use lazy_static::lazy_static;
use libc::{c_char, c_int, gid_t, passwd, size_t, uid_t};
use regex::Regex;
use std::collections::hash_map::DefaultHasher;
use std::ffi::CString;
use std::hash::{Hash, Hasher};
use std::ptr;

// NSS status codes
const NSS_STATUS_TRYAGAIN: c_int = -2;
const NSS_STATUS_UNAVAIL: c_int = -1;
const NSS_STATUS_NOTFOUND: c_int = 0;
const NSS_STATUS_SUCCESS: c_int = 1;

// UID range for VM users: 5000-65000
const UID_MIN: u32 = 5000;
const UID_RANGE: u32 = 60000;

// Default GID for VM users (nogroup)
const DEFAULT_GID: gid_t = 65534;

// VM user shell path
const VM_SHELL: &str = "/usr/local/bin/vm-ssh-proxy";

// VM user home directory
const VM_HOME: &str = "/nonexistent";

// VM user GECOS field
const VM_GECOS: &str = "Warlock VM";

lazy_static! {
    /// Regex pattern for valid VM usernames
    /// Matches: vm-{UUID v4}
    /// Example: vm-03c3f47c-c865-48e8-8b50-5dcd5c642dce
    static ref VM_USERNAME_PATTERN: Regex = Regex::new(
        r"^vm-[a-f0-9]{8}-[a-f0-9]{4}-4[a-f0-9]{3}-[89ab][a-f0-9]{3}-[a-f0-9]{12}$"
    ).unwrap();
}

/// Check if a username matches the VM user pattern
fn is_vm_user(username: &str) -> bool {
    VM_USERNAME_PATTERN.is_match(username)
}

/// Generate a deterministic UID from a username using hash function
fn generate_uid(username: &str) -> uid_t {
    let mut hasher = DefaultHasher::new();
    username.hash(&mut hasher);
    let hash = hasher.finish();

    // Map hash to UID range: 5000-65000
    UID_MIN + ((hash % UID_RANGE as u64) as u32)
}

/// Helper function to copy a string into a buffer
/// Returns true on success, false if buffer too small
unsafe fn copy_string_to_buffer(
    src: &str,
    buffer: *mut c_char,
    buflen: size_t,
    offset: &mut usize,
) -> Option<*mut c_char> {
    let bytes = src.as_bytes();
    let needed = bytes.len() + 1; // +1 for null terminator

    if *offset + needed > buflen {
        return None;
    }

    let dest = buffer.add(*offset);
    ptr::copy_nonoverlapping(bytes.as_ptr() as *const c_char, dest, bytes.len());
    *dest.add(bytes.len()) = 0; // null terminator

    *offset += needed;
    Some(dest)
}

/// Fill a passwd struct for a VM user
unsafe fn fill_passwd_struct(
    username: &str,
    pwd: *mut passwd,
    buffer: *mut c_char,
    buflen: size_t,
) -> c_int {
    let mut offset = 0;

    // Copy username
    let pw_name = match copy_string_to_buffer(username, buffer, buflen, &mut offset) {
        Some(ptr) => ptr,
        None => return NSS_STATUS_TRYAGAIN,
    };

    // Copy password placeholder
    let pw_passwd = match copy_string_to_buffer("x", buffer, buflen, &mut offset) {
        Some(ptr) => ptr,
        None => return NSS_STATUS_TRYAGAIN,
    };

    // Copy GECOS
    let pw_gecos = match copy_string_to_buffer(VM_GECOS, buffer, buflen, &mut offset) {
        Some(ptr) => ptr,
        None => return NSS_STATUS_TRYAGAIN,
    };

    // Copy home directory
    let pw_dir = match copy_string_to_buffer(VM_HOME, buffer, buflen, &mut offset) {
        Some(ptr) => ptr,
        None => return NSS_STATUS_TRYAGAIN,
    };

    // Copy shell
    let pw_shell = match copy_string_to_buffer(VM_SHELL, buffer, buflen, &mut offset) {
        Some(ptr) => ptr,
        None => return NSS_STATUS_TRYAGAIN,
    };

    // Fill passwd struct
    (*pwd).pw_name = pw_name;
    (*pwd).pw_passwd = pw_passwd;
    (*pwd).pw_uid = generate_uid(username);
    (*pwd).pw_gid = DEFAULT_GID;
    (*pwd).pw_gecos = pw_gecos;
    (*pwd).pw_dir = pw_dir;
    (*pwd).pw_shell = pw_shell;

    NSS_STATUS_SUCCESS
}

/// NSS function: Look up user by name
///
/// This is called by getpwnam() and similar functions
#[no_mangle]
pub unsafe extern "C" fn _nss_warlock_getpwnam_r(
    name: *const c_char,
    pwd: *mut passwd,
    buffer: *mut c_char,
    buflen: size_t,
    errnop: *mut c_int,
) -> c_int {
    // Convert C string to Rust string
    if name.is_null() {
        *errnop = libc::EINVAL;
        return NSS_STATUS_UNAVAIL;
    }

    let c_str = match std::ffi::CStr::from_ptr(name).to_str() {
        Ok(s) => s,
        Err(_) => {
            *errnop = libc::EINVAL;
            return NSS_STATUS_UNAVAIL;
        }
    };

    // Check if username matches VM pattern
    if !is_vm_user(c_str) {
        return NSS_STATUS_NOTFOUND;
    }

    // Fill passwd struct
    fill_passwd_struct(c_str, pwd, buffer, buflen)
}

/// NSS function: Look up user by UID
///
/// This is called by getpwuid() for reverse lookups
/// Since we can't reverse the hash, we return NOTFOUND
#[no_mangle]
pub unsafe extern "C" fn _nss_warlock_getpwuid_r(
    _uid: uid_t,
    _pwd: *mut passwd,
    _buffer: *mut c_char,
    _buflen: size_t,
    _errnop: *mut c_int,
) -> c_int {
    // We can't reverse-lookup VM users from UID
    // This is acceptable for our use case
    NSS_STATUS_NOTFOUND
}

/// NSS function: Initialize enumeration of users
///
/// We don't enumerate VM users (there could be infinite)
#[no_mangle]
pub unsafe extern "C" fn _nss_warlock_setpwent() -> c_int {
    NSS_STATUS_SUCCESS
}

/// NSS function: Get next user in enumeration
///
/// Always returns NOTFOUND since we don't enumerate
#[no_mangle]
pub unsafe extern "C" fn _nss_warlock_getpwent_r(
    _pwd: *mut passwd,
    _buffer: *mut c_char,
    _buflen: size_t,
    _errnop: *mut c_int,
) -> c_int {
    NSS_STATUS_NOTFOUND
}

/// NSS function: Close enumeration
#[no_mangle]
pub unsafe extern "C" fn _nss_warlock_endpwent() -> c_int {
    NSS_STATUS_SUCCESS
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vm_username_pattern_valid() {
        assert!(is_vm_user("vm-03c3f47c-c865-48e8-8b50-5dcd5c642dce"));
        assert!(is_vm_user("vm-12345678-1234-4abc-9def-123456789abc"));
        assert!(is_vm_user("vm-aaaaaaaa-bbbb-4ccc-addd-eeeeeeeeeeee"));
    }

    #[test]
    fn test_vm_username_pattern_invalid() {
        // Wrong prefix
        assert!(!is_vm_user("user-03c3f47c-c865-48e8-8b50-5dcd5c642dce"));

        // No UUID
        assert!(!is_vm_user("vm-invalid"));

        // Wrong UUID format (version must be 4)
        assert!(!is_vm_user("vm-03c3f47c-c865-38e8-8b50-5dcd5c642dce"));

        // Wrong UUID format (variant must be 8, 9, a, or b)
        assert!(!is_vm_user("vm-03c3f47c-c865-48e8-7b50-5dcd5c642dce"));

        // Uppercase letters
        assert!(!is_vm_user("vm-03C3F47C-C865-48E8-8B50-5DCD5C642DCE"));

        // Missing hyphens
        assert!(!is_vm_user("vm-03c3f47cc86548e88b505dcd5c642dce"));

        // Regular username
        assert!(!is_vm_user("bastionuser"));

        // Empty string
        assert!(!is_vm_user(""));
    }

    #[test]
    fn test_uid_generation_deterministic() {
        let uid1 = generate_uid("vm-03c3f47c-c865-48e8-8b50-5dcd5c642dce");
        let uid2 = generate_uid("vm-03c3f47c-c865-48e8-8b50-5dcd5c642dce");
        assert_eq!(uid1, uid2, "UID generation must be deterministic");
    }

    #[test]
    fn test_uid_generation_different_users() {
        let uid1 = generate_uid("vm-03c3f47c-c865-48e8-8b50-5dcd5c642dce");
        let uid2 = generate_uid("vm-12345678-1234-4abc-9def-123456789abc");
        assert_ne!(uid1, uid2, "Different users should have different UIDs");
    }

    #[test]
    fn test_uid_in_valid_range() {
        for username in &[
            "vm-03c3f47c-c865-48e8-8b50-5dcd5c642dce",
            "vm-12345678-1234-4abc-9def-123456789abc",
            "vm-aaaaaaaa-bbbb-4ccc-addd-eeeeeeeeeeee",
            "vm-00000000-0000-4000-8000-000000000000",
            "vm-ffffffff-ffff-4fff-bfff-ffffffffffff",
        ] {
            let uid = generate_uid(username);
            assert!(
                uid >= UID_MIN && uid < UID_MIN + UID_RANGE,
                "UID {} for {} is outside valid range {}-{}",
                uid,
                username,
                UID_MIN,
                UID_MIN + UID_RANGE - 1
            );
        }
    }

    #[test]
    fn test_uid_distribution() {
        // Generate UIDs for many users to check distribution
        let mut uids = std::collections::HashSet::new();

        for i in 0..1000 {
            let username = format!("vm-{:08x}-0000-4000-8000-000000000000", i);
            let uid = generate_uid(&username);
            uids.insert(uid);
        }

        // We should have good distribution (close to 1000 unique UIDs)
        // Allow some collisions due to hash function
        assert!(
            uids.len() > 950,
            "Expected > 950 unique UIDs, got {}",
            uids.len()
        );
    }
}
