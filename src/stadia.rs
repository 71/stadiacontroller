use std::{
    ffi::{c_void, CStr},
    fs::{File, OpenOptions},
    io::{Error, Write},
    mem::size_of,
    os::windows::prelude::{AsRawHandle, OpenOptionsExt},
    path::PathBuf,
    ptr::{null, null_mut},
    time::Duration,
};

use anyhow::Context;
use tokio::sync::oneshot;
use windows::{
    core::PCSTR,
    Win32::{
        Devices::{
            DeviceAndDriverInstallation::{
                SetupDiDestroyDeviceInfoList, SetupDiEnumDeviceInterfaces, SetupDiGetClassDevsA,
                SetupDiGetDeviceInterfaceDetailA, SetupDiGetDeviceRegistryPropertyA,
                DIGCF_DEVICEINTERFACE, DIGCF_PRESENT, SPDRP_HARDWAREID, SP_DEVICE_INTERFACE_DATA,
                SP_DEVICE_INTERFACE_DETAIL_DATA_A, SP_DEVINFO_DATA,
            },
            HumanInterfaceDevice::GUID_DEVINTERFACE_HID,
        },
        Foundation::{
            CloseHandle, BOOLEAN, ERROR_DEVICE_NOT_CONNECTED, ERROR_IO_PENDING, HANDLE, HWND,
        },
        Storage::FileSystem::{
            ReadFile, WriteFile, FILE_FLAG_OVERLAPPED, FILE_SHARE_READ, FILE_SHARE_WRITE,
        },
        System::{
            Threading::{
                CreateEventA, RegisterWaitForSingleObject, ResetEvent, UnregisterWait,
                WT_EXECUTEINWAITTHREAD, WT_EXECUTEONLYONCE,
            },
            IO::{CancelIo, GetOverlappedResult, OVERLAPPED},
        },
    },
};

/// A handle to a Stadia controller.
pub struct Controller(Option<AcquiredController>);

impl Controller {
    /// Creates a new [`Controller`] which is not connected to any device.
    pub const fn new() -> Self {
        Controller(None)
    }

    /// Reads a new report sent by the controller.
    pub async fn read_report(&mut self) -> anyhow::Result<Report> {
        loop {
            // Obtain inner controller.
            let inner = match &mut self.0 {
                Some(inner) => inner,
                None => loop {
                    match AcquiredController::acquire()
                        .context("cannot connect to Stadia controller")?
                    {
                        Some(inner) => {
                            self.0 = Some(inner);

                            break unsafe { self.0.as_mut().unwrap_unchecked() };
                        }
                        None => {
                            tokio::time::sleep(Duration::from_secs(1)).await;
                        }
                    }
                },
            };

            // Read from device; a report is expected to have a size of 11.
            let mut buf = [0; 512];
            let read_bytes =
                read_overlapped(inner.device_handle(), &mut inner.overlapped, &mut buf)
                    .await
                    .context("cannot read report from Stadia controller")?;

            let read_bytes = match read_bytes {
                Some(read_bytes) => read_bytes,
                None => {
                    // Controller was disconnected; re-acquire it.
                    self.0 = None;

                    continue;
                }
            };
            let start = if buf[0] == 0 { 1 } else { 0 };

            return Report::try_from(&buf[start..read_bytes as usize]);
        }
    }

    /// Makes the controller vibrate.
    pub fn vibrate(&mut self, large_motor: u8, small_motor: u8) -> anyhow::Result<()> {
        if let Some(inner) = &mut self.0 {
            write_overlapped(
                inner.device_handle(),
                &[0x05, large_motor, large_motor, small_motor, small_motor],
            )?;
        }

        Ok(())
    }
}

/// A [`Controller`] that was actually acquired.
struct AcquiredController {
    device: File,
    overlapped: OVERLAPPED,
}

impl AcquiredController {
    /// Connects to a Stadia device and returns an [`AcquiredController`]
    /// representing it.
    fn acquire() -> anyhow::Result<Option<Self>> {
        const STADIA_CONTROLLER_VENDOR_ID: u16 = 0x18D1;
        const STADIA_CONTROLLER_PRODUCT_ID: u16 = 0x9400;

        let device_path = match find_device_with_vid_and_pid(
            STADIA_CONTROLLER_VENDOR_ID,
            STADIA_CONTROLLER_PRODUCT_ID,
        )? {
            Some(path) => path,
            None => return Ok(None),
        };

        let device = OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(FILE_SHARE_READ.0 | FILE_SHARE_WRITE.0)
            .custom_flags(FILE_FLAG_OVERLAPPED.0)
            .open(device_path)
            .context("cannot open connection to Stadia controller")?;

        let read_event = unsafe { CreateEventA(null(), false, false, PCSTR::default())? };
        let overlapped = OVERLAPPED {
            hEvent: read_event,
            ..Default::default()
        };

        Ok(Some(AcquiredController { device, overlapped }))
    }

    /// Returns the Windows handle to the underlying Stadia device.
    fn device_handle(&self) -> HANDLE {
        HANDLE(self.device.as_raw_handle() as usize as isize)
    }
}

impl Drop for AcquiredController {
    fn drop(&mut self) {
        unsafe {
            CancelIo(self.device_handle());
            CloseHandle(self.overlapped.hEvent);
        }
    }
}

/// A report sent by the [`Controller`].
#[derive(Default)]
pub struct Report {
    pub vigem_report: vigem::XUSBReport,
    pub is_assistant_pressed: bool,
    pub is_capture_pressed: bool,
}

impl Report {
    /// Sets the given `button` if `bits` is not zero.
    #[inline]
    fn maybe_set_button(&mut self, button: vigem::XButton, bits: u8) {
        if bits != 0 {
            self.vigem_report.w_buttons |= button;
        }
    }

    /// Sets the given button.
    #[inline]
    fn set_button(&mut self, button: vigem::XButton) {
        self.vigem_report.w_buttons |= button;
    }

    #[inline]
    fn convert_axis_value(value: u8) -> i32 {
        let value = value as i32;
        let value = value << 8 | ((value << 1) & 0b1111);

        if value == 0xfffe {
            0xffff
        } else {
            value
        }
    }
}

impl TryFrom<&'_ [u8]> for Report {
    type Error = anyhow::Error;

    fn try_from(data: &'_ [u8]) -> anyhow::Result<Self> {
        if data.len() < 10 || data[0] != 0x03 {
            anyhow::bail!("unknown report format; raw report was {data:?}");
        }

        let mut report = Self::default();

        // Update buttons.
        let (dpad, b0, b1) = (data[1], data[2], data[3]);

        report.maybe_set_button(vigem::XButton::A, b1 & 0b0100_0000);
        report.maybe_set_button(vigem::XButton::B, b1 & 0b0010_0000);
        report.maybe_set_button(vigem::XButton::X, b1 & 0b0001_0000);
        report.maybe_set_button(vigem::XButton::Y, b1 & 0b0000_1000);
        report.maybe_set_button(vigem::XButton::LeftShoulder, b1 & 0b0000_0100);
        report.maybe_set_button(vigem::XButton::RightShoulder, b1 & 0b0000_0010);
        report.maybe_set_button(vigem::XButton::LeftThumb, b1 & 0b0000_0001);
        report.maybe_set_button(vigem::XButton::RightThumb, b0 & 0b1000_0000);
        report.maybe_set_button(vigem::XButton::Back, b0 & 0b0100_0000);
        report.maybe_set_button(vigem::XButton::Start, b0 & 0b0010_0000);
        report.maybe_set_button(vigem::XButton::Guide, b0 & 0b0001_0000);

        report.is_assistant_pressed = (b0 & 0b0000_0001) != 0;
        report.is_capture_pressed = (b0 & 0b0000_0010) != 0;

        // Update DPad.
        match dpad {
            0 => {
                report.set_button(vigem::XButton::DpadUp);
            }
            1 => {
                report.set_button(vigem::XButton::DpadUp);
                report.set_button(vigem::XButton::DpadRight);
            }
            2 => {
                report.set_button(vigem::XButton::DpadRight);
            }
            3 => {
                report.set_button(vigem::XButton::DpadRight);
                report.set_button(vigem::XButton::DpadDown);
            }
            4 => {
                report.set_button(vigem::XButton::DpadDown);
            }
            5 => {
                report.set_button(vigem::XButton::DpadDown);
                report.set_button(vigem::XButton::DpadLeft);
            }
            6 => {
                report.set_button(vigem::XButton::DpadLeft);
            }
            7 => {
                report.set_button(vigem::XButton::DpadLeft);
                report.set_button(vigem::XButton::DpadUp);
            }
            8 => (),
            _ => anyhow::bail!("unknown dpad value in report: {dpad}"),
        }

        // Normalize and convert axes values.
        // Port of https://github.com/MWisBest/StadiEm.
        let mut axes = [data[4], data[5], data[6], data[7]];

        for axis in &mut axes {
            if *axis <= 0x7F && *axis > 0x00 {
                *axis -= 1;
            }
        }

        let thumb_lx = Self::convert_axis_value(axes[0]) - 0x8000;
        let mut thumb_ly = -Self::convert_axis_value(axes[1]) + 0x7fff;
        let thumb_rx = Self::convert_axis_value(axes[2]) - 0x8000;
        let mut thumb_ry = -Self::convert_axis_value(axes[3]) + 0x7fff;

        for value in [&mut thumb_ly, &mut thumb_ry] {
            if *value == -1 {
                *value = 0;
            }
        }

        // Set axes values.
        report.vigem_report.s_thumb_lx = thumb_lx as i16;
        report.vigem_report.s_thumb_ly = thumb_ly as i16;
        report.vigem_report.s_thumb_rx = thumb_rx as i16;
        report.vigem_report.s_thumb_ry = thumb_ry as i16;

        // Set triggers.
        report.vigem_report.b_left_trigger = data[8];
        report.vigem_report.b_right_trigger = data[9];

        Ok(report)
    }
}

/// Returns the path to the first device with the given `vid` and `pid`, or
/// [`None`] if no such device can be found.
fn find_device_with_vid_and_pid(vid: u16, pid: u16) -> anyhow::Result<Option<PathBuf>> {
    // Compute expected hardware ID first.
    let expected_hardware_id = {
        let mut buffer = [0u8; 21];

        write!(buffer.as_mut_slice(), "HID\\VID_{vid:04X}&PID_{pid:04X}")
            .expect("expected_hardware_id is large enough to write result");

        buffer
    };

    // Enumerate over all devices looking for that hardware ID.
    let device_info_set = unsafe {
        SetupDiGetClassDevsA(
            &GUID_DEVINTERFACE_HID,
            PCSTR::default(),
            HWND::default(),
            DIGCF_PRESENT | DIGCF_DEVICEINTERFACE,
        )
    }?;

    scopeguard::defer! {
        unsafe {
            SetupDiDestroyDeviceInfoList(device_info_set);
        }
    }

    const BUF_SIZE: usize = 4096;
    let mut path_buffer = [0u8; BUF_SIZE];
    let mut device_interface_detail_data =
        path_buffer.as_mut_ptr() as *mut SP_DEVICE_INTERFACE_DETAIL_DATA_A;

    unsafe {
        (*device_interface_detail_data).cbSize =
            size_of::<SP_DEVICE_INTERFACE_DETAIL_DATA_A>() as _;
    }

    for device_idx in 0.. {
        // Read device interface data, necessary to read device interface detail
        // data below.
        let mut device_interface_data = SP_DEVICE_INTERFACE_DATA {
            cbSize: size_of::<SP_DEVICE_INTERFACE_DATA>() as _,
            ..Default::default()
        };

        let found_device_interface_data = unsafe {
            SetupDiEnumDeviceInterfaces(
                device_info_set,
                null(),
                &GUID_DEVINTERFACE_HID,
                device_idx,
                &mut device_interface_data,
            )
        };

        if !found_device_interface_data.as_bool() {
            // There are no more devices in the `device_info_set`.
            break;
        }

        // Read device info data and device interface detail data; the former is
        // used to obtain the `hardware_id`, and the latter is used to read the
        // device path if the `hardware_id` matches `expected_hardware_id`.
        let mut device_info_data = SP_DEVINFO_DATA {
            cbSize: size_of::<SP_DEVINFO_DATA>() as _,
            ..Default::default()
        };

        let mut device_interface_detail_data_required_size = BUF_SIZE as u32;

        let found_device_interface_detail_data = unsafe {
            SetupDiGetDeviceInterfaceDetailA(
                device_info_set,
                &device_interface_data,
                device_interface_detail_data,
                device_interface_detail_data_required_size,
                &mut device_interface_detail_data_required_size,
                &mut device_info_data,
            )
        };

        // If the `device_interface_data` was found, then we should be able to
        // find the corresponding `device_interface_detail_data`. If we can't,
        // that's an error in the given parameters.
        assert!(found_device_interface_detail_data.as_bool());

        if device_interface_detail_data_required_size > BUF_SIZE as u32
            || device_interface_detail_data_required_size == 0
        {
            // Path is invalid / unreadable.
            continue;
        }

        // Ensure this is the device we're looking for by comparing the hardware
        // ID.
        //
        // 512 bytes is more than enough for the Stadia controller (whose
        // required size was 154 when testing this function).
        let mut hardware_id = [0u8; 512];
        let mut hardware_id_required_size = hardware_id.len() as _;

        unsafe {
            SetupDiGetDeviceRegistryPropertyA(
                device_info_set,
                &device_info_data,
                SPDRP_HARDWAREID,
                null_mut(),
                hardware_id.as_mut_ptr(),
                hardware_id_required_size,
                &mut hardware_id_required_size,
            );
        }

        if !hardware_id.starts_with(&expected_hardware_id) {
            // Not the device we're looking for.
            continue;
        }

        // The hardware ID corresponds, now we read the zero-terminated path to
        // the device.
        let path = unsafe {
            CStr::from_ptr(
                &(*device_interface_detail_data).DevicePath[0].0 as *const u8 as *const i8,
            )
        };
        let path = path
            .to_str()
            .context("cannot convert device path to utf-8")?;

        return Ok(Some(PathBuf::from(path)));
    }

    Ok(None)
}

/// Reads once from `file` asynchronously with the given `overlapped` context,
/// and returns the number of read bytes. If the file is no longer available,
/// [`None`] will be returned and the file should be dropped.
async fn read_overlapped(
    handle: HANDLE,
    overlapped: &mut OVERLAPPED,
    buf: &mut [u8],
) -> anyhow::Result<Option<usize>> {
    unsafe extern "system" fn done_waiting(ctx: *mut c_void, _: BOOLEAN) {
        let tx = Box::from_raw(ctx as *mut oneshot::Sender<()>);
        let _ = tx.send(());
    }

    // Reset current event.
    unsafe {
        ResetEvent(overlapped.hEvent);
    }

    // Start read from file; the buffer will be kept by Windows and will be
    // filled asynchronously.
    let success = unsafe {
        ReadFile(
            handle,
            buf.as_mut_ptr() as *mut _,
            buf.len() as _,
            null_mut(),
            overlapped,
        )
    };

    anyhow::ensure!(
        success.as_bool() || Error::last_os_error().raw_os_error() == Some(ERROR_IO_PENDING.0 as _),
        "cannot read from device: {}",
        Error::last_os_error(),
    );

    // Start wait for the end of the read; completion will be sent to the
    // current function through a `oneshot` channel.
    let (tx, rx) = oneshot::channel();
    let mut wait_handle = HANDLE(0);

    let success = unsafe {
        RegisterWaitForSingleObject(
            &mut wait_handle,
            overlapped.hEvent,
            Some(done_waiting),
            Box::into_raw(Box::new(tx)) as *mut c_void,
            u32::MAX,
            WT_EXECUTEINWAITTHREAD | WT_EXECUTEONLYONCE,
        )
    };

    anyhow::ensure!(
        success.as_bool(),
        "cannot register for read completion: {}",
        Error::last_os_error()
    );

    scopeguard::defer! {
        unsafe {
            UnregisterWait(wait_handle);
        }
    }

    // If the current read is cancelled (e.g. because a vibration event is
    // received), the `rx.await` below will return, calling `UnregisterWait`
    // above. Re-assuing `ReadFile` later will not make us lose any reports.
    rx.await?;

    // Wait completed, we can query the number of read bytes (knowing that the
    // buffer was written to).
    let mut read_bytes = 0;
    let success = unsafe { GetOverlappedResult(handle, overlapped, &mut read_bytes, false) };

    if !success.as_bool()
        && Error::last_os_error().raw_os_error() == Some(ERROR_DEVICE_NOT_CONNECTED.0 as _)
    {
        // Device was disconnected.
        return Ok(None);
    }

    anyhow::ensure!(
        success.as_bool(),
        "cannot read overlapped result: {}",
        Error::last_os_error()
    );

    assert_ne!(read_bytes, 0);

    Ok(Some(read_bytes as _))
}

/// Writes `data` to `file` (with `file` an overlapped file).
fn write_overlapped(handle: HANDLE, data: &[u8]) -> anyhow::Result<()> {
    let mut overlapped = OVERLAPPED::default();

    // Start writing.
    let success = unsafe {
        WriteFile(
            handle,
            data.as_ptr() as *const c_void,
            data.len() as _,
            null_mut(),
            &mut overlapped,
        )
    };

    anyhow::ensure!(
        success.as_bool() || Error::last_os_error().raw_os_error() == Some(ERROR_IO_PENDING.0 as _),
        "cannot write to device: {}",
        Error::last_os_error(),
    );

    // Wait until we're done writing.
    let mut written_bytes = 0;

    let success = unsafe { GetOverlappedResult(handle, &overlapped, &mut written_bytes, true) };

    anyhow::ensure!(
        success.as_bool(),
        "cannot write to device: {}",
        Error::last_os_error()
    );

    Ok(())
}
