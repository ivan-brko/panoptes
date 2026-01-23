# Installation Guide

Panoptes is currently distributed as source code and must be built from source.

## Prerequisites

### Rust

Panoptes requires Rust 1.70 or later.

**Check if Rust is installed:**
```bash
rustc --version
# Should show rustc 1.70.0 or higher
```

**Install Rust (if needed):**
```bash
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh
source $HOME/.cargo/env
```

### Claude Code

Panoptes is designed to manage Claude Code sessions. You'll need Claude Code installed and configured.

**Install Claude Code:**
Visit [claude.ai/code](https://claude.ai/code) for installation instructions.

**Verify installation:**
```bash
claude --version
```

### Git

Git is required for repository operations and worktree management.

```bash
git --version
# Should show git version 2.x or higher
```

## Build from Source

### Clone the Repository

```bash
git clone https://github.com/ivan-brko/panoptes.git
cd panoptes
```

### Build Release Binary

```bash
cargo build --release
```

This creates an optimized binary at `./target/release/panoptes`.

The release build uses Link-Time Optimization (LTO) for better performance, so it may take a few minutes to compile.

### Build Debug Binary (Optional)

For development or troubleshooting:

```bash
cargo build
```

Creates an unoptimized binary at `./target/debug/panoptes`.

## Run Panoptes

### Direct Execution

```bash
./target/release/panoptes
```

### Add to PATH

To run from anywhere:

```bash
# Option 1: Copy to a directory in your PATH
sudo cp ./target/release/panoptes /usr/local/bin/

# Option 2: Add to your shell profile
echo 'export PATH="$PATH:/path/to/panoptes/target/release"' >> ~/.bashrc
source ~/.bashrc
```

Then run:
```bash
panoptes
```

### Using Cargo

You can also run directly with cargo:

```bash
cargo run --release
```

## First Run

When Panoptes starts for the first time:

1. It creates the `~/.panoptes/` directory and subdirectories
2. An empty configuration is initialized with default values
3. The hook server starts on port 9999

### Add Your First Project

1. Press `n` to add a new project
2. Enter the path to a git repository (e.g., `/home/user/projects/myapp`)
3. Press `Enter` to confirm

Panoptes will:
- Detect the repository and its default branch
- Create a branch entry for the default branch
- Display the project in the Projects Overview

### Create Your First Session

1. Navigate to a project (use `Enter` to open)
2. Navigate to a branch
3. Press `n` to create a new session
4. Enter a name (e.g., "frontend-work")
5. Press `Enter` to start

Claude Code will launch in the session.

## Updating

To update to the latest version:

```bash
cd panoptes
git pull
cargo build --release
```

## Uninstallation

### Remove the Binary

```bash
# If copied to /usr/local/bin
sudo rm /usr/local/bin/panoptes

# Or remove from wherever you placed it
```

### Remove Data (Optional)

```bash
# This removes all Panoptes data including projects and settings
rm -rf ~/.panoptes/
```

## Troubleshooting Installation

### Compilation Errors

If you encounter compilation errors:

1. Ensure Rust is up to date:
   ```bash
   rustup update
   ```

2. Clean and rebuild:
   ```bash
   cargo clean
   cargo build --release
   ```

### Missing Dependencies

On some systems, you may need additional development libraries:

**Ubuntu/Debian:**
```bash
sudo apt-get install build-essential pkg-config libssl-dev
```

**macOS:**
```bash
xcode-select --install
```

**Fedora:**
```bash
sudo dnf install gcc pkg-config openssl-devel
```

### Port Already in Use

If port 9999 is in use, see [Troubleshooting Guide](TROUBLESHOOTING.md#port-9999-already-in-use).

## Next Steps

After installation:

1. Read the [Keyboard Reference](KEYBOARD_REFERENCE.md) to learn the shortcuts
2. Review the [Configuration Guide](CONFIG_GUIDE.md) to customize settings
3. Check [Product Overview](PRODUCT.md) for feature details
