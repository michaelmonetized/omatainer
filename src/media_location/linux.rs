//! Read-only discovery through kernel mountinfo and the installed libudev API.
//! https://www.kernel.org/doc/html/latest/filesystems/proc.html
//! https://github.com/systemd/systemd/blob/main/src/libudev/libudev.h
use super::*;
use libc::{c_char, c_int, c_void, dev_t};
use std::{ffi::CStr, fs::File, io::Read, os::unix::fs::FileTypeExt};

const MOUNT_BYTES: usize = 2 * 1024 * 1024;

#[link(name = "udev")]
extern "C" {
    fn udev_new() -> *mut c_void;
    fn udev_unref(context: *mut c_void) -> *mut c_void;
    fn udev_enumerate_new(context: *mut c_void) -> *mut c_void;
    fn udev_enumerate_unref(enumeration: *mut c_void) -> *mut c_void;
    fn udev_enumerate_add_match_subsystem(enumeration: *mut c_void, name: *const c_char) -> c_int;
    fn udev_enumerate_scan_devices(enumeration: *mut c_void) -> c_int;
    fn udev_enumerate_get_list_entry(enumeration: *mut c_void) -> *mut c_void;
    fn udev_list_entry_get_next(entry: *mut c_void) -> *mut c_void;
    fn udev_list_entry_get_name(entry: *mut c_void) -> *const c_char;
    fn udev_device_new_from_syspath(context: *mut c_void, path: *const c_char) -> *mut c_void;
    fn udev_device_unref(device: *mut c_void) -> *mut c_void;
    fn udev_device_get_devnum(device: *mut c_void) -> dev_t;
    fn udev_device_get_property_value(device: *mut c_void, key: *const c_char) -> *const c_char;
    fn udev_device_get_sysattr_value(device: *mut c_void, key: *const c_char) -> *const c_char;
    fn udev_device_get_parent(device: *mut c_void) -> *mut c_void;
    fn udev_device_get_subsystem(device: *mut c_void) -> *const c_char;
}
struct Context(*mut c_void);
impl Drop for Context {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                udev_unref(self.0);
            }
        }
    }
}
struct Enumeration(*mut c_void);
impl Drop for Enumeration {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                udev_enumerate_unref(self.0);
            }
        }
    }
}
struct BlockDevice(*mut c_void);
impl Drop for BlockDevice {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                udev_device_unref(self.0);
            }
        }
    }
}

// libudev strings and borrowed parent pointers live at least until the owned
// device is released. Copy the selected properties within that ownership scope.
unsafe fn property(device: *mut c_void, key: &'static [u8]) -> Option<String> {
    let value = udev_device_get_property_value(device, key.as_ptr().cast());
    if value.is_null() {
        return None;
    }
    let bytes = CStr::from_ptr(value).to_bytes();
    if bytes.len() > 128 {
        return None;
    }
    std::str::from_utf8(bytes).ok().map(str::to_owned)
}
unsafe fn equals(value: *const c_char, expected: &[u8]) -> bool {
    !value.is_null() && CStr::from_ptr(value).to_bytes() == expected
}
fn blocks() -> Result<Vec<Block>, Failure> {
    // No contexts or udev object references cross a thread or callback.
    unsafe {
        let context = Context(udev_new());
        if context.0.is_null() {
            return Err(Failure::Unreadable("udev context unavailable".into()));
        }
        let enumeration = Enumeration(udev_enumerate_new(context.0));
        if enumeration.0.is_null() {
            return Err(Failure::Unreadable("udev enumeration unavailable".into()));
        }
        if udev_enumerate_add_match_subsystem(enumeration.0, b"block\0".as_ptr().cast()) < 0
            || udev_enumerate_scan_devices(enumeration.0) < 0
        {
            return Err(Failure::Unreadable("udev block discovery failed".into()));
        }
        let mut entry = udev_enumerate_get_list_entry(enumeration.0);
        let mut result = Vec::new();
        let mut visited = 0;
        while !entry.is_null() {
            if visited == MAX_BLOCKS {
                return Err(Failure::Capacity);
            }
            visited += 1;
            let path = udev_list_entry_get_name(entry);
            if path.is_null() {
                return Err(Failure::Changed);
            }
            let device = BlockDevice(udev_device_new_from_syspath(context.0, path));
            if device.0.is_null() {
                return Err(Failure::Changed);
            }
            let number = udev_device_get_devnum(device.0);
            if number != 0 {
                let uuid = property(device.0, b"ID_FS_UUID\0").unwrap_or_default();
                if !uuid.is_empty() && !valid_uuid(&uuid) {
                    return Err(Failure::Invalid);
                }
                let mut parent = device.0;
                let mut removable = false;
                for _ in 0..32 {
                    if parent.is_null() {
                        break;
                    }
                    removable |= property(parent, b"ID_BUS\0")
                        .is_some_and(|s| matches!(s.as_str(), "usb" | "firewire"))
                        || equals(udev_device_get_subsystem(parent), b"usb")
                        || equals(
                            udev_device_get_sysattr_value(parent, b"removable\0".as_ptr().cast()),
                            b"1",
                        );
                    parent = udev_device_get_parent(parent);
                }
                if !parent.is_null() {
                    return Err(Failure::Capacity);
                }
                result.push(Block {
                    device: Device::from_raw(number),
                    uuid,
                    removable,
                });
            }
            entry = udev_list_entry_get_next(entry);
        }
        Ok(result)
    }
}
fn namespace() -> Result<(u64, u64), Failure> {
    let metadata = std::fs::metadata("/proc/self/ns/mnt").map_err(io)?;
    Ok((metadata.dev(), metadata.ino()))
}
pub(super) fn discover() -> Result<Snapshot, Failure> {
    let before = namespace()?;
    let mut bytes = Vec::new();
    File::open("/proc/self/mountinfo")
        .map_err(io)?
        .take((MOUNT_BYTES + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(io)?;
    if bytes.len() > MOUNT_BYTES {
        return Err(Failure::Capacity);
    }
    let mut mounts = parse_mounts(&bytes)?;
    let blocks = blocks()?;
    for mount in &mut mounts {
        // Most block filesystems report the block's dev_t directly. Btrfs and
        // some stacked filesystems instead report an anonymous filesystem
        // device; their kernel-reported source must be an actual block node.
        mount.block = blocks
            .iter()
            .find(|b| b.device == mount.device)
            .map(|b| b.device);
        if mount.block.is_none() && mount.source.is_absolute() {
            if let Ok(metadata) = mount.source.metadata() {
                if metadata.file_type().is_block_device() {
                    let device = Device::from_raw(metadata.rdev());
                    mount.block = blocks.iter().find(|b| b.device == device).map(|b| b.device);
                }
            }
        }
    }
    let mut after = Vec::new();
    File::open("/proc/self/mountinfo")
        .map_err(io)?
        .take((MOUNT_BYTES + 1) as u64)
        .read_to_end(&mut after)
        .map_err(io)?;
    if namespace()? != before || after != bytes {
        return Err(Failure::Changed);
    }
    Ok(Snapshot {
        namespace: before,
        mounts,
        blocks,
    })
}
