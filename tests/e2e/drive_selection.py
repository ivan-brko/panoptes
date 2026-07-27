#!/usr/bin/env python3
"""Drive Panoptes' in-session mouse selection against the real binary.

The harness plays the part iTerm2 plays: it owns a PTY with Panoptes on the
far end, answers the startup queries Panoptes sends its terminal, and sends
SGR mouse reports exactly as a terminal does once crossterm has enabled mouse
capture. Assertions are on the *system clipboard*, so a passing run exercises
the whole chain

    iTerm(=harness) -> panoptes -> vt100 extraction -> pbcopy

Scenarios (argv[1], default `shell`):

    shell   drag, click, double/triple click, backwards and multi-row drags,
            edge auto-scroll, and a child that takes the mouse
    codex   the same selection over a Codex session, whose wheel events route
            differently from a shell's
    osc52   the fallback path: no working clipboard helper on PATH, so the
            copy has to leave as an OSC 52 sequence

`codex` needs a real, authenticated `~/.codex`. Only its credentials are
copied into the fake HOME - never symlinked, which would let a run rewrite the
developer's own config. It also spends one Codex turn.

Run it through the integration test, which builds the binary first and passes
its path in `PANOPTES_BIN`:

    cargo test --test selection_e2e -- --ignored

Running this file directly works too, as long as `target/debug/panoptes` is
not stale - `cargo test` does not rebuild it.

The developer's clipboard is saved on entry and restored on exit.
"""
import os, pty, sys, time, select, signal, subprocess, shutil, re, base64
import fcntl, termios, struct, tempfile, atexit

SCENARIO = sys.argv[1] if len(sys.argv) > 1 else "shell"
REPO_ROOT = os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
BIN = os.environ.get("PANOPTES_BIN", os.path.join(REPO_ROOT, "target/debug/panoptes"))
REAL_HOME = os.path.expanduser("~")

if not os.path.exists(BIN):
    sys.exit(f"panoptes binary not found at {BIN}; run `cargo build` first")

# Everything this run writes goes to a temp dir - never into the repo
SCRATCH = tempfile.mkdtemp(prefix="panoptes-e2e-")
HOME = os.path.join(SCRATCH, "home")
REPO = os.path.join(SCRATCH, "repo")
LOG = os.path.join(SCRATCH, f"{SCENARIO}.log")

ROWS, COLS = 40, 120

# How far back from live output the session view says the reader is. It used
# to be the title of the box drawn around the content; the box is gone and it
# lives in the header suffix now, beside the session name.
SCROLL_INDICATOR = r"\[↑(\d+)\]"

os.makedirs(HOME)
os.makedirs(REPO)
os.makedirs(os.path.join(HOME, ".panoptes"))
with open(os.path.join(HOME, ".panoptes", "config.toml"), "w") as f:
    # The user's live Panoptes holds 9999; never contend for it
    f.write("hook_port = 9871\n")
subprocess.run(["git", "init", "-q", "-b", "main"], cwd=REPO, check=True)
subprocess.run(["git", "-c", "user.email=t@t", "-c", "user.name=t",
                "commit", "-q", "--allow-empty", "-m", "init"], cwd=REPO, check=True)

if SCENARIO == "codex":
    # Borrow the developer's Codex *credentials* and nothing else.
    #
    # Symlinking the whole `~/.codex` would be far easier and is what an
    # earlier harness did - and it is destructive. Panoptes rewrites
    # `$CODEX_HOME/config.toml` to install its notify hook, and Codex records
    # trusted projects and session rollouts there, so every run would edit the
    # developer's real config: chaining a notify command that points at this
    # run's temp directory, which is deleted seconds later. Runs nest, and the
    # damage survives the test.
    #
    # So: a real scratch directory, with only the auth files copied in.
    real_codex = os.path.join(REAL_HOME, ".codex")
    if not os.path.exists(os.path.join(real_codex, "auth.json")):
        sys.exit("the codex scenario needs an authenticated ~/.codex")
    scratch_codex = os.path.join(HOME, ".codex")
    os.makedirs(scratch_codex)
    for name in ("auth.json", "installation_id"):
        source = os.path.join(real_codex, name)
        if os.path.exists(source):
            shutil.copy(source, os.path.join(scratch_codex, name))

# The OSC 52 scenario needs every clipboard helper to fail. Shadowing `pbcopy`
# with a shim that exits non-zero is enough: the others are not on macOS at
# all, so the fallback is the only path left.
child_path = os.environ.get("PATH", "")
if SCENARIO == "osc52":
    fakebin = os.path.join(SCRATCH, "fakebin")
    os.makedirs(fakebin)
    shim = os.path.join(fakebin, "pbcopy")
    with open(shim, "w") as f:
        f.write("#!/bin/sh\ncat >/dev/null\nexit 1\n")
    os.chmod(shim, 0o755)
    child_path = fakebin + os.pathsep + child_path


def pbpaste():
    return subprocess.run(["pbpaste"], capture_output=True).stdout.decode()


def pbcopy(text):
    subprocess.run(["pbcopy"], input=text.encode(), check=True)


ORIGINAL_CLIPBOARD = pbpaste()
atexit.register(lambda: pbcopy(ORIGINAL_CLIPBOARD))

try:
    import pyte
except ImportError:
    sys.exit("this harness needs `pyte` (pip install pyte)")

pid, fd = pty.fork()
if pid == 0:
    # This script may itself run inside a Claude Code session; leaked agent
    # markers make a nested agent think it is an unauthenticated child
    for var in list(os.environ):
        if var.startswith(("CLAUDE", "ANTHROPIC", "CODEX", "MCP", "OTEL")):
            del os.environ[var]
    os.environ["HOME"] = HOME
    os.environ["PATH"] = child_path
    os.environ["TERM"] = "xterm-256color"
    os.environ["COLORTERM"] = "truecolor"
    os.environ["TERM_PROGRAM"] = "iTerm.app"
    os.environ["TERM_PROGRAM_VERSION"] = "3.5.14"
    os.environ["SHELL"] = "/bin/zsh"
    os.execvp(BIN, [BIN])

fcntl.ioctl(fd, termios.TIOCSWINSZ, struct.pack("HHHH", ROWS, COLS, 0, 0))

log = open(LOG, "wb")
captured = b""
_screen = pyte.Screen(COLS, ROWS)
_stream = pyte.ByteStream(_screen)
_fed = 0
snapshots = []


def rows():
    global _fed
    _stream.feed(captured[_fed:])
    _fed = len(captured)
    return [l.rstrip() for l in _screen.display]


def screen_text():
    return "\n".join(rows())


def snapshot(label):
    snapshots.append((label, screen_text()))


def answer_host_queries(chunk):
    """Play iTerm2: answer the queries panoptes sends to its terminal."""
    if b"\x1b]10;?" in chunk:
        os.write(fd, b"\x1b]10;rgb:e4e4/e4e4/e4e4\x1b\\")
    if b"\x1b]11;?" in chunk:
        os.write(fd, b"\x1b]11;rgb:1212/1313/1414\x1b\\")
    if b"\x1b[>0q" in chunk:
        os.write(fd, b"\x1bP>|iTerm2 3.5.14\x1b\\")
    if b"\x1b[5n" in chunk:
        os.write(fd, b"\x1b[0n")
    # crossterm's keyboard-enhancement detection: kitty query (+ DA1 chaser)
    if b"\x1b[?u" in chunk:
        os.write(fd, b"\x1b[?0u")
    if re.search(rb"\x1b\[0?c", chunk):
        os.write(fd, b"\x1b[?62;4c")


def drain(seconds):
    global captured
    end = time.time() + seconds
    while time.time() < end:
        r, _, _ = select.select([fd], [], [], 0.1)
        if fd in r:
            try:
                chunk = os.read(fd, 65536)
            except OSError:
                return False
            if not chunk:
                return False
            captured += chunk
            log.write(chunk)
            log.flush()
            answer_host_queries(chunk)
    return True


def send(data, wait=0.6):
    os.write(fd, data if isinstance(data, bytes) else data.encode())
    drain(wait)


def wait_for(pattern, timeout=20.0, label=None):
    """Drain until the rendered screen matches `pattern` (regex)."""
    end = time.time() + timeout
    while time.time() < end:
        drain(0.4)
        if re.search(pattern, screen_text()):
            if label:
                snapshot(label)
            return True
    if label:
        snapshot(label + " (TIMEOUT)")
    return False


def find(needle, last=False):
    """1-based (row, col) of `needle` on the rendered screen, iTerm-style."""
    lines = list(enumerate(rows()))
    if last:
        lines.reverse()
    for r, line in lines:
        c = line.find(needle)
        if c >= 0:
            return r + 1, c + 1
    raise AssertionError(f"{needle!r} not on screen:\n{screen_text()}")


# SGR mouse reports, exactly as a terminal sends them once capture is on
def press(row, col, wait=0.4):
    send(f"\x1b[<0;{col};{row}M".encode(), wait)


def motion(row, col, wait=0.15):
    send(f"\x1b[<32;{col};{row}M".encode(), wait)


def release(row, col, wait=0.6):
    send(f"\x1b[<0;{col};{row}m".encode(), wait)


def wheel_up(row, col, wait=0.4):
    send(f"\x1b[<64;{col};{row}M".encode(), wait)


def wheel_down(row, col, wait=0.4):
    send(f"\x1b[<65;{col};{row}M".encode(), wait)


def to_live_view(row=1, col=1):
    """Scroll back down to live output the way a user now has to.

    There are no keyboard scroll keys in a session any more - every key but
    Esc belongs to the agent - so `End` and `PageDown` no longer come back
    here, they type into the agent. The wheel is the way.
    """
    for _ in range(60):
        if re.search(SCROLL_INDICATOR, screen_text()) is None:
            return True
        wheel_down(row, col, 0.1)
    return re.search(SCROLL_INDICATOR, screen_text()) is None


def drag(row, from_col, to_col):
    press(row, from_col)
    step = 1 if to_col >= from_col else -1
    stride = step * max(1, abs(to_col - from_col) // 4)
    for col in range(from_col + step, to_col + step, stride):
        motion(row, col)
    motion(row, to_col)
    release(row, to_col)


def multi_click(row, col, times):
    """`times` presses inside the double-click window (400 ms)."""
    for _ in range(times):
        press(row, col, 0.06)
        release(row, col, 0.06)
    drain(0.6)


ok = True


def check(name, cond):
    global ok
    print(("PASS " if cond else "FAIL ") + name)
    ok = ok and bool(cond)
    return cond


def open_branch():
    """Add the scratch repo as a project and drill into its branch."""
    drain(3.0)
    check("dashboard rendered", "Settings" in screen_text())
    send("n", 0.5)
    send(REPO, 0.5)
    send("\r", 1.0)
    send("\r", 2.5)
    send("\r", 1.5)   # open project
    send("\r", 1.5)   # open branch


def open_shell_session(marker="FIDELITY-42 SENTINEL"):
    send("s", 0.8)
    send("\r", 2.5)
    check("session view opened", "[SH]" in screen_text())
    send(f"echo {marker}\r", 2.0)
    check("command ran through emulation", wait_for(re.escape(marker), 8.0, "shell output"))


# =========================================================================
# shell: the whole selection surface over a plain shell session
# =========================================================================
def scenario_shell():
    open_branch()
    open_shell_session()
    check("footer offers a plain drag (no agent owns the mouse)",
          "drag: copy" in screen_text() and "⌥drag" not in screen_text())

    row, col = find("FIDELITY-42 SENTINEL")

    pbcopy("CLIPBOARD-UNTOUCHED")
    drag(row, col, col + len("FIDELITY-42") - 1)
    snapshot("after drag")
    pasted = pbpaste()
    check(f"drag copied the selection (got {pasted!r})", pasted == "FIDELITY-42")

    pbcopy("CLIPBOARD-UNTOUCHED")
    press(row, col)
    release(row, col)
    drain(0.5)
    check("a plain click leaves the clipboard alone", pbpaste() == "CLIPBOARD-UNTOUCHED")

    pbcopy("CLIPBOARD-UNTOUCHED")
    multi_click(row, col + 3, 2)
    pasted = pbpaste()
    check(f"double click took the whole word (got {pasted!r})", pasted == "FIDELITY-42")

    pbcopy("CLIPBOARD-UNTOUCHED")
    multi_click(row, col + 3, 3)
    pasted = pbpaste()
    check(f"triple click took the whole line (got {pasted!r})",
          "FIDELITY-42 SENTINEL" in pasted and pasted.count("\n") == 0)

    pbcopy("CLIPBOARD-UNTOUCHED")
    drag(row, col + len("FIDELITY-42") - 1, col)
    pasted = pbpaste()
    check(f"a right-to-left drag selects the same text (got {pasted!r})", pasted == "FIDELITY-42")

    # The wheel still drives local scrollback after a selection
    send("seq 1 200\r", 2.5)
    wheel_up(row, col)
    wheel_up(row, col)
    snapshot("wheel after selection")
    check("the wheel still drives local scrollback",
          re.search(SCROLL_INDICATOR, screen_text()) is not None)
    check("the wheel comes back to live output", to_live_view())

    # A drag across rows copies every line it covers
    send("printf 'ALPHA\\nBETA\\nGAMMA\\n'\r", 2.0)
    if check("three lines printed", wait_for(r"GAMMA", 8.0, "three lines")):
        # The echoed command line holds these words too: take the output
        arow, acol = find("ALPHA", last=True)
        grow, gcol = find("GAMMA", last=True)
        pbcopy("CLIPBOARD-UNTOUCHED")
        press(arow, acol)
        motion(arow + 1, gcol)
        motion(grow, gcol + 4)
        release(grow, gcol + 4)
        drain(0.6)
        pasted = pbpaste()
        check(f"a multi-row drag copies every line (got {pasted!r})",
              pasted.splitlines()[:1] == ["ALPHA"] and "GAMMA" in pasted)

    # Dragging past the top edge scrolls and keeps selecting. Drag events stop
    # arriving when the pointer stops moving, so this is the tick-driven half.
    send("seq 1 400\r", 3.0)
    check("history generated", wait_for(r"\b400\b", 8.0, "long history"))
    pbcopy("CLIPBOARD-UNTOUCHED")
    press(ROWS - 4, 3)
    motion(1, 3)
    drain(1.0)
    snapshot("edge auto-scroll")
    scrolled = re.search(SCROLL_INDICATOR, screen_text())
    release(1, 3)
    drain(0.5)
    lines = pbpaste().splitlines()
    check(f"holding past the top edge scrolled the view "
          f"(indicator {scrolled and scrolled.group(1)})",
          scrolled is not None and int(scrolled.group(1)) > 5)
    check(f"the selection grew past one screenful ({len(lines)} lines)", len(lines) > ROWS)
    check("back to live output", to_live_view())

    # A drag that began as a double click keeps taking whole words, including
    # while the edge is scrolling the view under a pointer that is not moving
    send("for i in $(seq 1 200); do echo \"AAA-$i BBB-$i\"; done\r", 3.0)
    if check("word-boundary history printed", wait_for(r"AAA-200", 8.0, "word history")):
        wrow, wcol = find("AAA-200", last=True)
        pbcopy("CLIPBOARD-UNTOUCHED")
        # Double click in the middle of a token, then hold past the top edge
        press(wrow, wcol + 2, 0.06)
        release(wrow, wcol + 2, 0.06)
        press(wrow, wcol + 2, 0.06)
        motion(1, wcol + 2)
        drain(0.8)
        release(1, wcol + 2)
        drain(0.5)
        pasted = pbpaste()
        tokens = pasted.split()
        broken = [t for t in tokens if not re.fullmatch(r"(AAA|BBB)-\d+", t)]
        check(f"an auto-scrolled word drag stops on word boundaries "
              f"({len(tokens)} tokens, first {tokens[:1]})",
              tokens and not broken)
    check("back to live output", to_live_view())

    # A child that takes the mouse keeps its own drags - what claude, vim and
    # htop do, without needing one of them here.
    #
    # Last in this scenario on purpose: the forwarded reports arrive at zsh as
    # literal escape bytes, which leaves its line editor in a state where
    # later commands do not reliably run.
    # One command, because the forwarded reports that follow leave zsh's line
    # editor unable to run another. `cat -v` renders the reports the child
    # receives as visible text, which is what lets the coordinates be checked.
    send("echo MOUSEOWNER; printf '\\033[?1000h\\033[?1006h'; cat -v\r", 2.0)
    check("footer defers to the child that took the mouse",
          "⌥drag: copy" in screen_text())
    pbcopy("CLIPBOARD-UNTOUCHED")
    mrow, mcol = find("MOUSEOWNER", last=True)
    drag(mrow, mcol, mcol + 5)
    check("a drag over a mouse-owning child does not copy",
          pbpaste() == "CLIPBOARD-UNTOUCHED")

    # ...and it lands on the cell under the pointer.
    #
    # The regression this guards (b0064d1, and again when the content area
    # stopped being inset for a border): Panoptes translates screen
    # coordinates into the child's, and if its idea of where the content
    # starts drifts from where it is drawn, every forwarded click lands off by
    # exactly that much - invisibly, because the click still goes somewhere.
    #
    # SGR columns are 1-based, so column 1 is the leftmost cell of the screen.
    # It is also the leftmost cell of the child, because the content runs to
    # the edge - there is no border in between to account for.
    def child_reports():
        return re.findall(r"\^\[\[<\d+;(\d+);(\d+)[Mm]", screen_text())

    before = len(child_reports())
    press(mrow, 1)
    release(mrow, 1)
    drain(0.8)
    snapshot("forwarded click at the left edge")
    seen = child_reports()
    if check(f"the child received a mouse report ({seen[-2:]})", len(seen) > before):
        left_col, left_row = (int(v) for v in seen[-1])
        check(f"a click on the screen's left edge reaches the child as its "
              f"own left edge (got column {left_col})", left_col == 1)

        # Nine columns right and one row up must arrive as exactly that.
        # Upwards on purpose: the rows below here are the footer, and a click
        # outside the content area is rightly not forwarded at all.
        press(mrow - 1, 10)
        release(mrow - 1, 10)
        drain(0.8)
        col_moved, row_moved = (int(v) for v in child_reports()[-1])
        check(f"the mapping stays linear (got column {col_moved}, row {row_moved} "
              f"from column {left_col}, row {left_row})",
              col_moved == left_col + 9 and row_moved == left_row - 1)
    send(b"\x04", 0.5)   # Ctrl+D out of cat


# =========================================================================
# codex: wheel events route before the PTY forward, unlike a shell's
# =========================================================================
def scenario_codex():
    open_branch()
    send("n", 0.8)
    send(b"\x1b[B", 0.5)   # selector down -> Codex
    send("\r", 1.0)
    send("\r", 3.0)
    if wait_for(r"Do you trust the contents", 12.0, "codex trust dialog"):
        send("\r", 2.0)
    # The banner Codex prints on startup, across versions
    banner = "OpenAI Codex"
    if not check("codex UI rendered",
                 wait_for(re.escape(banner), 40.0, "codex ui")):
        return

    # PAN-15 routes Codex to Panoptes' own selection because Codex does not
    # ask for mouse reporting. If that ever changes, this is where it shows.
    check("codex leaves the mouse to us", "⌥drag: copy" not in screen_text())

    row, col = find(banner)
    pbcopy("CLIPBOARD-UNTOUCHED")
    drag(row, col, col + len(banner) - 1)
    pasted = pbpaste()
    check(f"a drag over a codex session copies (got {pasted!r})", pasted == banner)

    # Wheel notches over a Codex session are handled before the PTY forward,
    # which is the ordering that makes its path different from a shell's. It
    # has to keep driving local scrollback rather than being swallowed by the
    # selection that just happened.
    #
    # That a session which has only just started already has history to scroll
    # into is itself the vendored vt100 patch working: Codex pins a footer with
    # a scroll region, and those lines now reach real scrollback.
    wheel_up(row, col)
    snapshot("codex wheel")
    check("the wheel still drives local scrollback for codex",
          re.search(SCROLL_INDICATOR, screen_text()) is not None)

    # Paging up past the oldest line must stop there (PAN-20).
    #
    # A Codex session has two histories - the vterm's scrollback and the
    # plain-text fallback buffer - and the bug was choosing between them by
    # asking "did the vterm advance?". That is false at the top of real
    # scrollback just as it is when there is none, so reaching the top used to
    # dump the reader into the shallower fallback from the live view: the
    # indicator climbed, then collapsed. Scrolling up moved the view down.
    #
    # Startup alone leaves about one row of scrollback, which would let this
    # pass without testing anything, so spend one Codex turn on real history.
    # Asking it to *print* rather than run a command keeps the turn inside the
    # reply, with no sandbox approval to answer.
    # Enter goes separately: sent in the same burst as the text, Codex's input
    # widget keeps it as part of the line and the prompt is never submitted.
    send("print the numbers 1 to 200, one per line, and nothing else", 1.5)
    send("\r", 1.0)
    if not check("codex produced a screenful of history",
                 wait_for(r"\b19[0-9]\b", 180.0, "codex line output")):
        return
    drain(3.0)

    check("back to live output before scrolling up", to_live_view())
    # The wheel, because there are no scroll keys in a session any more - a
    # PgUp here types into Codex.
    offsets = []
    for _ in range(120):
        wheel_up(row, col, 0.06)
        seen = re.search(SCROLL_INDICATOR, screen_text())
        offsets.append(int(seen.group(1)) if seen else 0)
    snapshot("codex scrolled to the top")
    dropped = [(a, b) for a, b in zip(offsets, offsets[1:]) if b < a]
    check(f"scrolling up never moves the view down (peak {max(offsets)}, "
          f"ended {offsets[-1]}, drops {dropped[:3]})",
          not dropped)
    check(f"scrolling up climbed real history, not one row (peak {max(offsets)})",
          max(offsets) > ROWS)
    check(f"scrolling up reached a top and stayed on it (last five {offsets[-5:]})",
          offsets[-1] == max(offsets))


# =========================================================================
# osc52: no clipboard helper works, so the copy leaves as an escape sequence
# =========================================================================
def scenario_osc52():
    open_branch()
    open_shell_session("OSC52-MARKER")
    row, col = find("OSC52-MARKER")

    pbcopy("CLIPBOARD-UNTOUCHED")
    before = len(captured)
    drag(row, col, col + len("OSC52-MARKER") - 1)
    drain(0.5)

    # `pbcopy` on PATH is a shim that fails, so nothing reached the system
    # clipboard and the sequence had to go to the terminal instead
    check("the failing helper did not reach the system clipboard",
          pbpaste() == "CLIPBOARD-UNTOUCHED")

    tail = captured[before:]
    match = re.search(rb"\x1b\]52;c;([A-Za-z0-9+/=]+)(?:\x07|\x1b\\)", tail)
    if check("an OSC 52 clipboard write reached the terminal", match is not None):
        payload = base64.b64decode(match.group(1)).decode(errors="replace")
        check(f"the OSC 52 payload is the selection (got {payload!r})",
              payload == "OSC52-MARKER")


# =========================================================================
# exited: a shell whose process is gone must say so (PAN-23)
# =========================================================================
def scenario_exited():
    open_branch()
    open_shell_session("EXIT-PROBE")

    send("exit\r", 2.0)
    # The header is the only thing that can say the process is gone: the
    # scrollback still reads as a shell sitting at a prompt.
    labelled = wait_for(r"- Exited", 15.0, "exited label")
    snapshot("after the shell exited")
    check("the header reports the shell exited", labelled)

    # And the process really is gone, so nothing echoes any more
    send("echo STILLALIVE\r", 1.5)
    check("a dead shell echoes nothing", "STILLALIVE" not in screen_text())


SCENARIOS = {
    "shell": scenario_shell,
    "exited": scenario_exited,
    "codex": scenario_codex,
    "osc52": scenario_osc52,
}
if SCENARIO not in SCENARIOS:
    sys.exit(f"unknown scenario {SCENARIO!r}; pick one of {', '.join(SCENARIOS)}")

try:
    SCENARIOS[SCENARIO]()
finally:
    # Teardown: leave the session, quit, confirm.
    #
    # Guarded, because a panoptes that has already exited makes every write
    # fail - and losing the screens and the log is exactly backwards when the
    # thing under test just died.
    try:
        send(b"\x1b", 0.5)
        send(b"\x1b", 0.5)
        send("q", 0.8)
        send("y", 1.2)
        time.sleep(0.5)
    except OSError as e:
        print(f"(panoptes was already gone at teardown: {e})")
    try:
        os.kill(pid, signal.SIGKILL)
    except ProcessLookupError:
        pass
    log.close()
    screens = os.path.join(SCRATCH, f"{SCENARIO}-screens.txt")
    with open(screens, "w") as f:
        for label, text in snapshots:
            f.write(f"\n===== {label} =====\n{text}\n")
    print(f"\nlog: {LOG}\nscreens: {screens}")
    if ok:
        shutil.rmtree(SCRATCH, ignore_errors=True)
    else:
        print(f"(kept {SCRATCH} for inspection)")

sys.exit(0 if ok else 1)
