# Warlock NSS Module

A custom NSS (Name Service Switch) module for dynamic VM user resolution in the Warlock Firecracker hosting platform.

## Overview

This NSS module allows SSH logins to Firecracker VMs using usernames like `vm-{uuid}` without pre-creating users in `/etc/passwd`. When SSH attempts to look up a VM user, this module dynamically returns user information, enabling seamless VM access through the bastion server.

## How It Works

```
User: ssh vm-03c3f47c-c865-48e8-8b50-5dcd5c642dce@bastion
  ↓
SSH Server: getpwnam("vm-03c3f47c-...")
  ↓
NSS Module: Pattern match → Return user info
  ↓
SSH: Authenticate with keys → Execute proxy script
  ↓
Proxy: Query gateway API → Connect to worker
  ↓
Connected to VM console
```

## Features

- **Dynamic user resolution** - No need to pre-create VM users
- **Pattern matching** - Accepts usernames matching `vm-{UUID v4}` format
- **Deterministic UIDs** - Same username always generates the same UID
- **UID collision resistance** - Hash-based UID generation across 60,000 value range
- **Safe implementation** - Written in Rust with FFI to C-compatible interface
- **Zero dependencies at runtime** - Compiled as standalone shared library

## Username Pattern

The module accepts usernames matching this pattern:

```
vm-{UUID v4}
```

Examples:
- `vm-03c3f47c-c865-48e8-8b50-5dcd5c642dce` ✓
- `vm-12345678-1234-4abc-9def-123456789abc` ✓
- `vm-invalid` ✗ (not a valid UUID)
- `user-03c3f47c-c865-48e8-8b50-5dcd5c642dce` ✗ (wrong prefix)

## User Details

For each valid VM username, the module returns:

| Field | Value |
|-------|-------|
| Username | `vm-{uuid}` |
| UID | Hash-based (5000-65000) |
| GID | 65534 (nogroup) |
| Home | `/nonexistent` |
| Shell | `/usr/local/bin/vm-ssh-proxy` |
| GECOS | "Warlock VM" |

## Installation

### Automated Installation (Recommended)

The bastion server's cloud-init automatically installs the NSS module. No manual steps required.

### Manual Installation

```bash
# Download latest release
VERSION="v0.1.0"
sudo curl -fsSL "https://github.com/TristanBlackwell/warlock-nss/releases/download/$VERSION/libnss_warlock.so.2" \
     -o /lib/x86_64-linux-gnu/libnss_warlock.so.2

# Set permissions
sudo chmod 644 /lib/x86_64-linux-gnu/libnss_warlock.so.2

# Update /etc/nsswitch.conf
sudo sed -i 's/^passwd:.*/passwd:         files warlock systemd/' /etc/nsswitch.conf

# Verify installation
getent passwd vm-00000000-0000-4000-8000-000000000000
```

Expected output:
```
vm-00000000-0000-4000-8000-000000000000:x:5000:65534:Warlock VM:/nonexistent:/usr/local/bin/vm-ssh-proxy
```

## Building from Source

### Prerequisites

- Rust toolchain (stable)
- Linux system (Ubuntu 24.04 recommended)

### Build Steps

```bash
# Clone repository
git clone https://github.com/TristanBlackwell/warlock-nss.git
cd warlock-nss

# Run tests
cargo test

# Build release
cargo build --release

# The compiled library is at:
# target/release/libnss_warlock.so
```

### Install Locally Built Library

```bash
sudo cp target/release/libnss_warlock.so /lib/x86_64-linux-gnu/libnss_warlock.so.2
sudo chmod 644 /lib/x86_64-linux-gnu/libnss_warlock.so.2
```

## Testing

### Unit Tests

```bash
cargo test
```

The test suite validates:
- Username pattern matching (valid/invalid formats)
- UID generation (deterministic, unique, in range)
- Hash distribution (collision resistance)

### Integration Tests

```bash
# Test valid VM username
getent passwd vm-03c3f47c-c865-48e8-8b50-5dcd5c642dce

# Test invalid username (should return nothing)
getent passwd invalid-user

# Test UID consistency
UID1=$(getent passwd vm-00000000-0000-4000-8000-000000000000 | cut -d: -f3)
UID2=$(getent passwd vm-00000000-0000-4000-8000-000000000000 | cut -d: -f3)
[ "$UID1" = "$UID2" ] && echo "UID is deterministic"

# Test shell
getent passwd vm-00000000-0000-4000-8000-000000000000 | cut -d: -f7
# Should output: /usr/local/bin/vm-ssh-proxy
```

### End-to-End Test

```bash
# SSH to a VM through bastion
ssh vm-03c3f47c-c865-48e8-8b50-5dcd5c642dce@bastion
```

## Troubleshooting

### Module not loading

```bash
# Check if module is installed
ls -l /lib/x86_64-linux-gnu/libnss_warlock.so.2

# Check nsswitch.conf
grep passwd /etc/nsswitch.conf
# Should include "warlock" in the passwd line

# Check for library errors
ldd /lib/x86_64-linux-gnu/libnss_warlock.so.2
```

### getent returns nothing

```bash
# Test with a valid UUID v4
getent passwd vm-00000000-0000-4000-8000-000000000000

# If still nothing, check NSS configuration
sudo strace -e openat getent passwd vm-00000000-0000-4000-8000-000000000000 2>&1 | grep warlock
```

### SSH login fails

```bash
# Check SSH logs
sudo journalctl -u ssh -f

# Verify proxy script exists
ls -l /usr/local/bin/vm-ssh-proxy

# Test proxy script manually
sudo -u bastionuser /usr/local/bin/vm-ssh-proxy
```

## Uninstallation

```bash
# Remove NSS module
sudo rm /lib/x86_64-linux-gnu/libnss_warlock.so.2

# Restore nsswitch.conf
sudo sed -i 's/^passwd:.*/passwd:         files systemd/' /etc/nsswitch.conf
```

## Architecture

### NSS Interface

The module implements these NSS functions:

- `_nss_warlock_getpwnam_r` - Look up user by name
- `_nss_warlock_getpwuid_r` - Look up user by UID (always returns NOTFOUND)
- `_nss_warlock_setpwent` - Initialize enumeration (no-op)
- `_nss_warlock_getpwent_r` - Get next user (always returns NOTFOUND)
- `_nss_warlock_endpwent` - Close enumeration (no-op)

### UID Generation

UIDs are generated using Rust's `DefaultHasher` (SipHash-2-4):

```rust
fn generate_uid(username: &str) -> uid_t {
    let mut hasher = DefaultHasher::new();
    username.hash(&mut hasher);
    let hash = hasher.finish();
    5000 + ((hash % 60000) as u32)
}
```

This provides:
- **Determinism**: Same username always generates same UID
- **Distribution**: Good spread across 60,000 value range
- **Collision resistance**: Low probability of UID conflicts

### Security Considerations

- **No privilege escalation**: VM users have no home directory or sudo access
- **Forced command**: SSH config forces execution of proxy script
- **Key-based auth**: Only authorized keys can authenticate
- **Isolated UIDs**: VM user UIDs don't conflict with system users (5000+)
- **No enumeration**: Module doesn't enumerate users (prevents user listing)

## Performance

- **Lookup time**: < 1ms (pattern matching + hash computation)
- **Memory usage**: < 1KB per lookup
- **No I/O**: All operations in-memory
- **No network calls**: No API dependencies

## Integration with Warlock Infrastructure

This NSS module integrates with:

1. **Bastion Server** - Resolves VM usernames for SSH login
2. **Gateway API** - Proxy script queries `/vm/{id}/location`
3. **Worker Nodes** - Final SSH connection to VM console
4. **Firecracker VMs** - Ultimate destination for user SSH sessions

See [Warlock Infrastructure](https://github.com/TristanBlackwell/warlock-infra) for complete setup.

## Development

### Project Structure

```
warlock-nss/
├── Cargo.toml           # Rust package configuration
├── src/
│   └── lib.rs          # Main NSS implementation
├── .github/
│   └── workflows/
│       └── release.yml # CI/CD pipeline
└── README.md           # This file
```

### Testing Changes

```bash
# Build and install locally
cargo build --release
sudo cp target/release/libnss_warlock.so /lib/x86_64-linux-gnu/libnss_warlock.so.2

# Test immediately
getent passwd vm-00000000-0000-4000-8000-000000000000
```

### Release Process

1. Update version in `Cargo.toml`
2. Commit changes: `git commit -am "Bump version to X.Y.Z"`
3. Tag release: `git tag vX.Y.Z`
4. Push: `git push && git push --tags`
5. GitHub Actions builds and creates release automatically

## Related Projects

- [Warlock](https://github.com/TristanBlackwell/warlock) - Firecracker control plane
- [Warlock Gateway](https://github.com/TristanBlackwell/warlock-gateway) - VM registry service
- [Warlock Infrastructure](https://github.com/TristanBlackwell/warlock-infra) - Terraform deployment

## License

MIT License - See [LICENSE](LICENSE) file for details

## Contributing

Contributions welcome! Please open an issue or PR.

## Support

For issues or questions:
- Open an issue on GitHub
- Contact: Tristan Blackwell
