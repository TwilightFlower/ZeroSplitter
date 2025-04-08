use std::io::{self, Read};

use bytemuck::{Pod, Zeroable};

#[derive(Debug, Default, Clone, Copy, Zeroable, Pod)]
#[repr(C)]
pub struct FrameData {
	pub score_p1: i32,
	pub score_p2: i32,
	pub stage: u8,
	pub game_loop: u8,
	pub checkpoint: u8,
	pub difficulty: i8,
}

impl FrameData {
	const SIZE: usize = size_of::<Self>();

	pub fn as_bytes(self) -> [u8; Self::SIZE] {
		bytemuck::cast(self)
	}

	pub fn from_bytes(bytes: [u8; Self::SIZE]) -> Self {
		bytemuck::cast(bytes)
	}

	pub fn read_from<R: Read>(read: &mut R) -> Result<Self, io::Error> {
		let mut buf = [0; Self::SIZE];
		read.read_exact(&mut buf)?;
		Ok(Self::from_bytes(buf))
	}

	pub fn is_first_stage(&self) -> bool {
		self.stage == 1 && self.game_loop == 0
	}

	pub fn is_menu(&self) -> bool {
		self.stage == 0
	}

	pub fn total_score(&self) -> i32 {
		self.score_p1 + self.score_p2
	}
}
