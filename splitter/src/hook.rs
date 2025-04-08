use std::{
	env::current_exe,
	ffi::{CStr, OsStr},
	fs, mem,
	os::windows::ffi::OsStrExt,
	thread,
	time::Duration,
};

use crate::system::{self, ProcessHandle};
use bytemuck::cast_slice;
use log::info;
use windows::{
	Win32::System::{
		LibraryLoader::{GetModuleHandleA, GetProcAddress},
		Memory::{MEM_COMMIT, PAGE_READWRITE},
		Threading::{
			LPTHREAD_START_ROUTINE, PROCESS_CREATE_THREAD, PROCESS_QUERY_INFORMATION,
			PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_VM_OPERATION, PROCESS_VM_READ, PROCESS_VM_WRITE,
		},
	},
	core::PCSTR,
};

pub fn hook_zeroranger() {
	let dll_path = fs::canonicalize(current_exe().expect("getting own path").with_file_name("payload.dll"))
		.expect("canonicalizing path");

	let zr_process = loop {
		if let Some(handle) = find_zeroranger() {
			break handle;
		}
		let _ = thread::sleep(Duration::from_secs(5));
	};

	let wide: Vec<u16> = dll_path.as_os_str().encode_wide().collect();

	let load_library = find_load_library_addr();
	inject_dll(&zr_process, &wide, load_library);
}

fn find_zeroranger() -> Option<ProcessHandle> {
	println!("Searching for process to hook...");
	for pid in system::list_processes().unwrap() {
		let handle = system::open_process(pid, PROCESS_QUERY_LIMITED_INFORMATION).unwrap();
		let exe = handle.get_executable();
		if exe.file_name() == Some(OsStr::new(&"ZeroRanger.exe")) {
			// we need a more privileged access to ZR
			let privileged_handle = system::open_process(
				pid,
				PROCESS_VM_OPERATION
					| PROCESS_VM_READ
					| PROCESS_VM_WRITE
					| PROCESS_CREATE_THREAD
					| PROCESS_QUERY_INFORMATION,
			)
			.expect("Acquiring privileged handle");
			info!("Found ZeroRanger with PID {}", pid);

			return Some(privileged_handle);
		}
	}

	None
}

fn find_load_library_addr() -> LPTHREAD_START_ROUTINE {
	unsafe {
		let handle = GetModuleHandleA(pcstr_of(c"kernel32.dll")).expect("getting kernel32.dll handle");
		let farproc = GetProcAddress(handle, pcstr_of(c"LoadLibraryW")).expect("no LoadLibraryW????");
		Some(mem::transmute(farproc))
	}
}

fn inject_dll(into: &ProcessHandle, path: &[u16], load_library: LPTHREAD_START_ROUTINE) {
	unsafe {
		let remote_path = into.virtual_alloc_ex(None, path.len() * 2 as usize, MEM_COMMIT, PAGE_READWRITE);
		into.write_memory(remote_path, cast_slice(path))
			.expect("Writing path to memory");
		into.create_remote_thread(None, load_library, Some(remote_path))
			.expect("Creating remote thread");
	}
}

fn pcstr_of(string: &'static CStr) -> PCSTR {
	PCSTR(string.as_ptr() as *const u8)
}
