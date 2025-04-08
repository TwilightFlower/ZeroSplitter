use std::{
	env,
	fs::{self, File},
	net::UdpSocket,
	sync::{
		OnceLock,
		mpsc::{self, Receiver, Sender},
	},
	thread,
	time::Duration,
};

use common::FrameData;
use eframe::{
	App, Frame, NativeOptions,
	egui::{
		Align, CentralPanel, Color32, ComboBox, Context, IconData, Id, Key, Layout, Sides, ThemePreference,
		ViewportBuilder, ViewportId,
	},
};
use log::{error, warn};
use serde::{Deserialize, Serialize};

mod hook;
mod system;
mod ui;

const DARK_GREEN: Color32 = Color32::from_rgb(0, 0x4f, 0x4d);
const GREEN: Color32 = Color32::from_rgb(0, 0x94, 0x79);
const LIGHT_ORANGE: Color32 = Color32::from_rgb(0xff, 0xc0, 0x73);
const DARK_ORANGE: Color32 = Color32::from_rgb(0xff, 0x80, 0);

static EGUI_CTX: OnceLock<Context> = OnceLock::new();

fn main() {
	pretty_env_logger::init();

	let options = NativeOptions {
		viewport: ViewportBuilder::default()
			.with_inner_size([300., 300.])
			.with_icon(IconData::default())
			.with_title("ZeroSplitter"),
		..Default::default()
	};

	let (tx, rx) = mpsc::channel();

	thread::spawn(|| ipc_thread(tx));

	eframe::run_native(
		"ZeroSplitter",
		options,
		Box::new(|c| {
			let _ = EGUI_CTX.set(c.egui_ctx.clone());
			c.egui_ctx.set_theme(ThemePreference::Dark);
			Ok(Box::new(ZeroSplitter::load(rx)))
		}),
	)
	.unwrap();
}

fn ipc_thread(channel: Sender<FrameData>) {
	let socket = UdpSocket::bind("127.0.0.1:23888").expect("Binding socket");
	socket
		.set_read_timeout(Some(Duration::from_secs(1)))
		.expect("Setting socket timeout");

	let mut buf = [0; size_of::<FrameData>()];
	loop {
		while socket.recv(&mut buf).is_ok() {
			let data = FrameData::from_bytes(buf);
			let _ = channel.send(data);
			if let Some(ctx) = EGUI_CTX.get() {
				ctx.request_repaint();
			}
		}
		// timed out, hook the game
		hook::hook_zeroranger();
	}
}

struct ZeroSplitter {
	categories: Vec<Category>,
	current_category: usize,
	data_source: Receiver<FrameData>,
	last_frame: FrameData,
	current_run: Run,
	current_split: Option<usize>,
	current_split_score_offset: i32,
	waiting_for_category: bool,
	waiting_for_rename: bool,
	waiting_for_confirm: bool,
	dialog_rx: Receiver<String>,
	dialog_tx: Sender<String>,
	comparison: Category,
}

impl ZeroSplitter {
	fn new(data_source: Receiver<FrameData>) -> Self {
		let (tx, rx) = mpsc::channel();
		let mut default_categories = Vec::new();
		default_categories.push(Category::new("Type-C".to_string()));
		default_categories.push(Category::new("Type-B".to_string()));
		Self {
			categories: default_categories,
			data_source,
			last_frame: Default::default(),
			current_category: 0,
			current_run: Default::default(),
			current_split: None,
			current_split_score_offset: 0,
			dialog_rx: rx,
			dialog_tx: tx,
			waiting_for_category: false,
			waiting_for_rename: false,
			waiting_for_confirm: false,
			comparison: Category::new("<null>".to_string()),
		}
	}

	fn load(data_source: Receiver<FrameData>) -> Self {
		let data_path = env::current_exe()
			.expect("Could not get program directory")
			.with_file_name("zs_data.json");

		match fs::exists(&data_path) {
			Ok(true) => (),
			Ok(false) => return Self::new(data_source),
			Err(e) => {
				warn!("Could not tell if data file exists: {}", e);
				return Self::new(data_source);
			}
		}

		match File::open(&data_path) {
			Ok(file) => {
				let data: Vec<Category> = serde_json::from_reader(file).expect("Loading data");
				if data.is_empty() {
					Self::new(data_source)
				} else {
					Self {
						current_category: 0,
						categories: data,
						..Self::new(data_source)
					}
				}
			}
			Err(e) => panic!("Could not open extant data file at {:?}: {}", &data_path, e),
		}
	}

	fn save_splits(&mut self) {
		self.categories[self.current_category].update_from_run(&self.current_run);

		let data_path = env::current_exe()
			.expect("Could not get program directory")
			.with_file_name("zs_data.json");
		let file = match File::create(&data_path) {
			Ok(file) => file,
			Err(err) => {
				error!("Could not save: Could not open data file {:?}: {}", &data_path, err);
				return;
			}
		};

		if let Err(err) = serde_json::to_writer_pretty(file, &self.categories) {
			error!("Error writing save: {}", err);
		}
	}

	fn update_frame(&mut self, frame: FrameData) {
		// WV not yet implemented, idk if it'll crash or not
		if frame.difficulty == 0 {
			// Reset if we just left the menu or returned to 1-1
			if frame.stage != self.last_frame.stage && (self.last_frame.is_menu() || frame.is_first_stage()) {
				self.reset();
			}

			if !frame.is_menu() {
				let frame_split = (frame.stage - 1 - frame.game_loop) as usize;

				// Split if necessary
				if frame.stage != self.last_frame.stage {
					self.current_split = Some(frame_split);
					self.current_split_score_offset = self.last_frame.total_score();
					self.save_splits();
				}

				// If our score got reset by a continue, fix the score offset.
				if self.current_split_score_offset > frame.total_score() {
					self.current_split_score_offset = 0;
				}

				// Update run and split scores
				self.current_run.score = frame.total_score();
				let split_score = frame.total_score() - self.current_split_score_offset;
				self.current_run.splits[frame_split] = split_score;
			} else {
				// End the run if we're back on the menu
				self.end_run();
			}
		}

		self.last_frame = frame;
	}

	fn reset(&mut self) {
		self.end_run();
		self.current_run = Default::default();
		self.comparison = self.categories[self.current_category].clone();
	}

	fn end_run(&mut self) {
		self.save_splits();
		self.current_split = None;
	}
}

impl App for ZeroSplitter {
	fn update(&mut self, ctx: &Context, _frame: &mut Frame) {
		while let Ok(data) = self.data_source.try_recv() {
			self.update_frame(data);
		}

		CentralPanel::default().show(ctx, |ui| {
			ui.with_layout(Layout::top_down_justified(Align::Min), |ui| {
				ui.horizontal(|ui| {
					ui.label("Category: ");
					ComboBox::from_label("").show_index(ui, &mut self.current_category, self.categories.len(), |i| {
						&self.categories[i].name
					});
					if ui.small_button("+").clicked() {
						self.waiting_for_category = true;
					}
					/*if ui.button("Delete").clicked() {
						self.waiting_for_confirm = true;
					}*/
					if ui.button("Rename").clicked() {
						self.waiting_for_rename = true;
					}
				});

				let cur_category = &self.categories[self.current_category];

				for (i, split) in self.current_run.splits.iter().enumerate() {
					let stage_n = (i & 3) + 1;
					let loop_n = (i >> 2) + 1;
					let best = cur_category.best_splits[i];
					let stored_best = self.comparison.personal_best.splits[i];
					let pb = self.comparison.personal_best.splits[i];

					Sides::new().show(
						ui,
						|left| {
							left.label(format!("{}-{}", loop_n, stage_n));

							if best > 0 {
								left.colored_label(GREEN, best.to_string());
							}
						},
						|right| {
							if *split != 0 || self.current_split == Some(i) {
								let split_color = if self.current_split == Some(i) {
									Color32::WHITE
								} else {
									DARK_ORANGE
								};

								right.colored_label(split_color, split.to_string());

								if self.current_split != Some(i) {
									// past split, we should show a diff
									let diff = *split - pb;
									let diff_color = if *split > stored_best {
										LIGHT_ORANGE
									} else if diff >= 0 {
										Color32::WHITE
									} else {
										DARK_GREEN
									};

									if diff > 0 {
										right.colored_label(diff_color, format!("+{}", diff));
									} else {
										right.colored_label(diff_color, diff.to_string());
									}
								}
							} else {
								right.colored_label(DARK_GREEN, "--");
							}
						},
					);
				}

				ui.label(format!("Personal Best: {}", cur_category.personal_best.score));
				ui.label(format!(
					"Sum of Best: {}",
					cur_category.best_splits.into_iter().sum::<i32>()
				))
			});
		});

		if self.waiting_for_category {
			if let Ok(new_category) = self.dialog_rx.try_recv() {
				if !new_category.is_empty() {
					self.categories.push(Category::new(new_category));
					self.current_category = self.categories.len() - 1;
					self.save_splits();
				}
				self.waiting_for_category = false;
			} else {
				entry_dialog(ctx, self.dialog_tx.clone(), "Enter new category name");
			}
		}

		if self.waiting_for_rename {
			if let Ok(new_name) = self.dialog_rx.try_recv() {
				if !new_name.is_empty() {
					self.categories[self.current_category].name = new_name;
					self.save_splits();
				}
				self.waiting_for_rename = false;
			} else {
				entry_dialog(ctx, self.dialog_tx.clone(), "Enter new name for category");
			}
		}

		if self.waiting_for_confirm {
			if let Ok(confirmation) = self.dialog_rx.try_recv() {
				if confirmation == "Deleted" {
					self.categories.remove(self.current_category);
					self.current_category = self.current_category.saturating_sub(1);
				}
				self.waiting_for_confirm = false;
			} else {
				confirm_dialog(
					ctx,
					self.dialog_tx.clone(),
					format!(
						"Are you sure you want to delete category {}?",
						self.categories[self.current_category].name
					),
				);
			}
		}
	}

	fn on_exit(&mut self, _gl: Option<&eframe::glow::Context>) {
		self.save_splits();
	}
}

fn entry_dialog(ctx: &Context, tx: Sender<String>, msg: &'static str) {
	let vp_builder = ViewportBuilder::default()
		.with_title("ZeroSplitter")
		.with_active(true)
		.with_resizable(false)
		.with_minimize_button(false)
		.with_maximize_button(false)
		.with_inner_size([200., 100.]);

	ctx.show_viewport_deferred(ViewportId::from_hash_of("entry dialog"), vp_builder, move |ctx, _| {
		if ctx.input(|input| input.viewport().close_requested()) {
			let _ = tx.send("".to_string());
			request_repaint();
			return;
		}

		let text_id = Id::new("edit text");
		let mut edit_str = ctx.data_mut(|data| data.get_temp_mut_or_insert_with(text_id, || String::new()).clone());

		CentralPanel::default().show(ctx, |ui| {
			ui.vertical_centered_justified(|ui| {
				ui.label(msg);
				if ui.text_edit_singleline(&mut edit_str).lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
					let _ = tx.send(edit_str.clone());
					request_repaint();
				}
			});
		});

		ctx.data_mut(|data| {
			data.insert_temp(text_id, edit_str);
		});
	});
}

fn confirm_dialog(ctx: &Context, tx: Sender<String>, msg: String) {
	let vp_builder = ViewportBuilder::default()
		.with_title("ZeroSplitter")
		.with_active(true)
		.with_resizable(false)
		.with_minimize_button(false)
		.with_maximize_button(false)
		.with_inner_size([200., 100.]);

	ctx.show_viewport_deferred(ViewportId::from_hash_of("confirm dialog"), vp_builder, move |ctx, _| {
		if ctx.input(|input| input.viewport().close_requested()) {
			let _ = tx.send("".to_string());
			request_repaint();
			return;
		}

		CentralPanel::default().show(ctx, |ui| {
			ui.vertical_centered_justified(|ui| {
				ui.label(msg.clone());
				ui.columns_const(|[left, right]| {
					if left.button("Delete").clicked() {
						let _ = tx.send("Deleted".to_string());
						request_repaint();
					} else if right.button("Cancel").clicked() {
						let _ = tx.send("".to_string());
						request_repaint();
					}
				});
			});
		});
	});
}

fn request_repaint() {
	if let Some(ctx) = EGUI_CTX.get() {
		ctx.request_repaint_after(Duration::from_millis(100));
	}
}

#[derive(Debug, Default, Serialize, Deserialize, Clone, Copy)]
struct Run {
	splits: [i32; 8],
	score: i32,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
struct Category {
	personal_best: Run,
	best_splits: [i32; 8],
	name: String,
}

impl Category {
	fn new(name: String) -> Self {
		Category {
			personal_best: Default::default(),
			best_splits: Default::default(),
			name,
		}
	}

	fn update_from_run(&mut self, run: &Run) {
		if run.score > self.personal_best.score {
			self.personal_best = *run;
		}

		for (best, new) in self.best_splits.iter_mut().zip(run.splits.iter()) {
			if *new > *best {
				*best = *new;
			}
		}
	}
}
