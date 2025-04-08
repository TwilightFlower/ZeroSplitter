use std::{
	arch::global_asm,
	ffi::{CStr, c_void},
	mem::MaybeUninit,
	net::UdpSocket,
	ptr,
	sync::{
		LazyLock,
		mpsc::{self, SyncSender},
	},
	thread,
};

use common::FrameData;
use windows::{
	Win32::{
		Foundation::{HINSTANCE, TRUE},
		System::{
			Memory::{PAGE_EXECUTE_READWRITE, VirtualProtect},
			SystemServices::DLL_PROCESS_ATTACH,
		},
	},
	core::BOOL,
};

const LOOP_CALL_ADDR: *mut [u8; 4] = 0x5db6b2 as _; // the actual address portion of the call to the tick function

static COMM_SENDER: LazyLock<Option<SyncSender<FrameData>>> = LazyLock::new(try_init_comm_thread);

fn try_init_comm_thread() -> Option<SyncSender<FrameData>> {
	let udp_socket = match UdpSocket::bind("127.0.0.1:0") {
		Ok(socket) => {
			if let Err(err) = socket.connect("127.0.0.1:23888") {
				eprintln!("[ZeroSplitter] Error connecting UDP socket: {}", err);
				None
			} else {
				Some(socket)
			}
		}
		Err(err) => {
			eprintln!("[ZeroSplitter] Error creating UDP socket: {}", err);
			None
		}
	}?;

	let (tx, rx) = mpsc::sync_channel::<FrameData>(0);

	let thread_builder = thread::Builder::new().name("ZeroSplitter communication thread".to_string());

	let thread_result = thread_builder.spawn(move || {
		while let Ok(frame_data) = rx.recv() {
			let _ = udp_socket.send(&frame_data.as_bytes());
		}
	});

	if let Err(e) = thread_result {
		eprintln!("[ZeroSplitter] Error spawning communication thread: {}", e);
		None
	} else {
		Some(tx)
	}
}

#[unsafe(no_mangle)]
pub unsafe extern "stdcall" fn DllMain(_instance: HINSTANCE, reason: u32, _reserved: *mut c_void) -> BOOL {
	if reason != DLL_PROCESS_ATTACH {
		return TRUE;
	}

	println!("[ZeroSplitter] Injecting hooks...");
	unsafe {
		let mut out = PAGE_EXECUTE_READWRITE;
		VirtualProtect(LOOP_CALL_ADDR.cast(), 4, PAGE_EXECUTE_READWRITE, &mut out).expect("VirtualProtect call");
		ptr::write(
			LOOP_CALL_ADDR,
			((redir_loop_to as usize) - (LOOP_CALL_ADDR as usize + 4)).to_le_bytes(),
		);
		// fix the page type
		VirtualProtect(LOOP_CALL_ADDR.cast(), 4, out, &mut out).expect("Returning page type");
	}
	println!("[ZeroSplitter] Injected hooks.");

	TRUE
}

unsafe extern "C" fn loop_callback() {
	if let Some(tx) = &*COMM_SENDER {
		let data = get_frame_data();
		let _ = tx.try_send(data);
	}
}

fn get_frame_data() -> FrameData {
	let smagic = read_var(c"smagic").unwrap().value;
	let score_p1 = read_var(c"score_one").unwrap().value - smagic;
	let score_p2 = read_var(c"score_two").unwrap().value - smagic;
	let game_loop = read_var(c"loop").unwrap().value as u8;
	let checkpoint = read_var(c"checkpoint").unwrap().value as u8;
	let difficulty = read_var(c"difficulty").unwrap().value as i8;
	let stage_p1 = read_var(c"stage_one").unwrap().value as u8;
	let stage_p2 = read_var(c"stage_two").unwrap().value as u8;
	let stage = stage_p1.max(stage_p2);
	FrameData {
		score_p1: score_p1 as i32,
		score_p2: score_p2 as i32,
		stage,
		game_loop,
		checkpoint,
		difficulty,
	}
}

fn read_var(name: &CStr) -> Option<GMVariable> {
	unsafe {
		let index = game_fn::get_global_var_index(name.as_ptr());
		if index >= 0 {
			let mut var_data: MaybeUninit<GMVariable> = MaybeUninit::zeroed();
			game_fn::read_var_into(index + 100000, (&mut var_data).as_mut_ptr().cast());
			Some(var_data.assume_init())
		} else {
			None
		}
	}
}

#[derive(Debug)]
#[repr(C)]
struct GMVariable {
	value: f64,
	unknown_1: u32,
	var_type: u32,
}

// Awful hack because allocating executable memory for a stub seems annoying, and arbitrary calling conventions exist.
// The underscore is because i686 windows targets mangle some names in extern blocks when building a cdylib.
// Don't ask how I know...
global_asm! {
	".global _redir_loop_to
	_redir_loop_to:
		push eax
		push ecx
		push edx
		call {callback}
		pop edx
		pop ecx
		pop eax
		jmp [abs_jump_hack]
	abs_jump_hack: .4byte {orig_addr}",
	orig_addr = const 0x4c5070,
	callback = sym loop_callback
}

unsafe extern "C" {
	fn redir_loop_to();
}

mod game_fn {
	use std::{
		ffi::{c_char, c_void},
		mem,
	};

	pub unsafe fn get_global_var_index(name: *const c_char) -> i32 {
		unsafe {
			let ptr: unsafe extern "C" fn(*const c_char) -> i32 = mem::transmute(0x53e2a0);
			ptr(name)
		}
	}

	pub unsafe fn read_var_into(var_index: i32, out: *mut c_void) -> bool {
		unsafe {
			let ptr: unsafe extern "C" fn(i32, u32, *mut c_void) -> bool = mem::transmute(0x40dcc0);
			ptr(var_index, 0x80000000, out)
		}
	}
}
