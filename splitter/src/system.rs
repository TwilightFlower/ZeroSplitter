use std::{
	ffi::{OsString, c_void},
	os::windows::ffi::OsStringExt,
	path::PathBuf,
	ptr, u32,
};

use windows::{
	Win32::{
		Foundation::{CloseHandle, HANDLE, HMODULE},
		Security::SECURITY_ATTRIBUTES,
		System::{
			Diagnostics::Debug::{ReadProcessMemory, WriteProcessMemory},
			Memory::{PAGE_PROTECTION_FLAGS, VIRTUAL_ALLOCATION_TYPE, VirtualAllocEx},
			ProcessStatus::{EnumProcessModules, EnumProcesses, GetModuleFileNameExW, GetProcessImageFileNameW},
			Threading::{CreateRemoteThread, LPTHREAD_START_ROUTINE, OpenProcess, PROCESS_ACCESS_RIGHTS},
		},
	},
	core::Error,
};

pub fn list_processes() -> Result<Vec<u32>, Error> {
	let mut buf_entries = 32;
	let mut read_bytes: u32 = u32::MAX;
	let mut buf = Vec::new();

	while read_bytes >= buf_entries * 4 {
		buf_entries *= 2;
		buf = vec![0; buf_entries as usize];
		unsafe {
			EnumProcesses(buf.as_mut_ptr(), buf_entries * 4, &mut read_bytes)?;
		}
	}

	buf.truncate(read_bytes as usize / 4);
	Ok(buf)
}

pub fn open_process(pid: u32, access: PROCESS_ACCESS_RIGHTS) -> Result<ProcessHandle, Error> {
	Ok(unsafe { ProcessHandle::from_raw(OpenProcess(access, false, pid)?) })
}

#[derive(Clone, Debug)]
pub struct ProcessHandle {
	raw: HANDLE,
}

impl ProcessHandle {
	pub unsafe fn from_raw(raw: HANDLE) -> Self {
		ProcessHandle { raw }
	}

	pub fn as_raw(&self) -> HANDLE {
		self.raw
	}

	pub fn get_executable(&self) -> PathBuf {
		let mut buffer = [0; 256];
		let size = unsafe {
			// this can error, but windows-rs doesn't expose it as a result for some reason.
			GetProcessImageFileNameW(self.raw, &mut buffer)
		};
		OsString::from_wide(&buffer[0..size as usize]).into()
	}

	pub unsafe fn read_memory(&self, base_addr: *const c_void, buffer: &mut [u8]) -> Result<(), Error> {
		unsafe { ReadProcessMemory(self.raw, base_addr, buffer.as_mut_ptr().cast(), buffer.len(), None) }
	}

	pub unsafe fn write_memory(&self, base_addr: *const c_void, buffer: &[u8]) -> Result<(), Error> {
		unsafe { WriteProcessMemory(self.raw, base_addr, buffer.as_ptr().cast(), buffer.len(), None) }
	}

	pub unsafe fn virtual_alloc_ex(
		&self,
		address: Option<*const c_void>,
		size: usize,
		alloc_type: VIRTUAL_ALLOCATION_TYPE,
		page_protection: PAGE_PROTECTION_FLAGS,
	) -> *mut c_void {
		unsafe { VirtualAllocEx(self.raw, address, size, alloc_type, page_protection) }
	}

	pub unsafe fn create_remote_thread(
		&self,
		security: Option<&SECURITY_ATTRIBUTES>,
		start: LPTHREAD_START_ROUTINE,
		param: Option<*const c_void>,
	) -> Result<HANDLE, Error> {
		let mut tid = 0;
		let res = unsafe {
			CreateRemoteThread(
				self.raw,
				security.map(ptr::from_ref),
				0,
				start,
				param,
				0,
				Some(&mut tid),
			)
		};
		res
	}

	pub fn enumerate_modules(&self) -> Vec<HMODULE> {
		let mut needed = 0;
		unsafe {
			EnumProcessModules(self.raw, ptr::null_mut(), 0, &mut needed).unwrap();
		}

		let mut modules = vec![HMODULE(ptr::null_mut()); needed as usize];
		unsafe {
			EnumProcessModules(
				self.raw,
				modules.as_mut_ptr(),
				needed * size_of::<HMODULE>() as u32,
				&mut needed,
			)
			.unwrap();
		}

		modules
	}

	pub fn get_module_file_name_ex(&self, module: HMODULE) -> OsString {
		let mut buffer = [0; 256];
		let size = unsafe { GetModuleFileNameExW(Some(self.raw), Some(module), &mut buffer) };
		OsString::from_wide(&buffer[0..size as usize])
	}
}

impl Drop for ProcessHandle {
	fn drop(&mut self) {
		if let Err(err) = unsafe { CloseHandle(self.raw) } {
			eprintln!("Error closing handle: {}", err);
		}
	}
}
