use windows::{
    Wdk::System::SystemServices::RtlGetVersion,
    Win32::System::SystemInformation::{OSVERSIONINFOEXW, OSVERSIONINFOW},
};

pub fn is_win11() -> bool {
    let mut info = OSVERSIONINFOW {
        dwOSVersionInfoSize: std::mem::size_of::<OSVERSIONINFOEXW>() as _,
        ..Default::default()
    };

    let status = unsafe { RtlGetVersion(&mut info) };
    status.is_ok() && info.dwBuildNumber >= 22000
}
