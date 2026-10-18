#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use eframe::egui::{self, Align, Color32, FontId, Key, Layout, RichText, TextEdit};
use eframe::{App, CreationContext, Frame, NativeOptions};
use rfd::FileDialog;
use serde::Deserialize;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::fs::OpenOptions;
use std::hash::{Hash, Hasher};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Instant, SystemTime, UNIX_EPOCH};

const MAX_FILE_BYTES: u64 = 2 * 1024 * 1024;
const EDITOR_BG: Color32 = Color32::from_rgb(17, 19, 24);
const PANEL_BG: Color32 = Color32::from_rgb(21, 23, 29);
const BORDER: Color32 = Color32::from_rgb(39, 42, 51);
const TEXT: Color32 = Color32::from_rgb(230, 231, 236);
const MUTED: Color32 = Color32::from_rgb(130, 135, 148);
const ACCENT: Color32 = Color32::from_rgb(117, 142, 255);
const ACTIVE: Color32 = Color32::from_rgb(47, 50, 77);
const TERMINAL_BG: Color32 = Color32::from_rgb(15, 17, 21);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum SidebarView {
    Explorer,
    Search,
    Extensions,
}

#[derive(Clone, Debug)]
struct ExtensionRunner {
    id: String,
    name: String,
    description: String,
    program: String,
    args: Vec<String>,
    file_extensions: Vec<String>,
}

#[derive(Debug, Deserialize, Default)]
struct ExtensionManifest {
    #[serde(default)]
    extensions: Vec<ManifestExtension>,
}

#[derive(Debug, Deserialize)]
struct ManifestExtension {
    id: String,
    name: String,
    #[serde(default)]
    description: String,
    program: String,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    file_extensions: Vec<String>,
}

#[derive(Clone, Debug)]
struct WorkspaceEntry {
    path: PathBuf,
    name: String,
    is_directory: bool,
}

#[derive(Clone, Debug)]
struct EditorTab {
    path: PathBuf,
    content: String,
    saved_content: String,
    saved_fingerprint: u64,
}

impl EditorTab {
    fn dirty(&self) -> bool {
        self.content != self.saved_content
    }

    fn name(&self) -> String {
        self.path
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("untitled")
            .to_owned()
    }
}

struct EnvironApp {
    workspace: Option<PathBuf>,
    directory_cache: HashMap<PathBuf, Vec<WorkspaceEntry>>,
    expanded: HashSet<PathBuf>,
    tabs: Vec<EditorTab>,
    active_tab: Option<usize>,
    command_palette_open: bool,
    command_query: String,
    sidebar_view: SidebarView,
    sidebar_visible: bool,
    terminal_visible: bool,
    search_query: String,
    extension_runners: Vec<ExtensionRunner>,
    terminal_input: String,
    terminal_lines: Vec<String>,
    terminal_receiver: Option<Receiver<String>>,
    terminal_running: bool,
    find_open: bool,
    find_query: String,
    replace_query: String,
    confirm_close: Option<usize>,
    about_open: bool,
    error: Option<String>,
    notice: String,
    saved_at: Option<Instant>,
}

struct LoadedFile {
    content: String,
    fingerprint: u64,
}

impl EnvironApp {
    fn new(_cc: &CreationContext<'_>) -> Self {
        Self {
            workspace: None,
            directory_cache: HashMap::new(),
            expanded: HashSet::new(),
            tabs: Vec::new(),
            active_tab: None,
            command_palette_open: false,
            command_query: String::new(),
            sidebar_view: SidebarView::Explorer,
            sidebar_visible: true,
            terminal_visible: true,
            search_query: String::new(),
            extension_runners: builtin_extensions(),
            terminal_input: String::new(),
            terminal_lines: vec![
                "Environ terminal — commands run inside the open workspace.".to_owned(),
            ],
            terminal_receiver: None,
            terminal_running: false,
            find_open: false,
            find_query: String::new(),
            replace_query: String::new(),
            confirm_close: None,
            about_open: false,
            error: None,
            notice: "Ready when you are".to_owned(),
            saved_at: None,
        }
    }

    fn open_workspace(&mut self) {
        let Some(folder) = FileDialog::new()
            .set_title("Open a project folder")
            .pick_folder()
        else {
            return;
        };

        if let Err(error) = self.set_workspace(folder) {
            self.show_error(error);
        }
    }

    fn set_workspace(&mut self, folder: PathBuf) -> Result<(), String> {
        let root =
            fs::canonicalize(folder).map_err(|error| format!("Could not open folder: {error}"))?;
        if !root.is_dir() {
            return Err("The selected path is not a folder.".to_owned());
        }

        let entries = read_directory(&root, &root)?;
        self.workspace = Some(root.clone());
        self.directory_cache.clear();
        self.directory_cache.insert(root.clone(), entries);
        self.expanded.clear();
        self.expanded.insert(root);
        self.tabs.clear();
        self.active_tab = None;
        self.extension_runners = load_extensions(self.workspace.as_deref());
        self.terminal_lines =
            vec!["Environ terminal — commands run inside the open workspace.".to_owned()];
        self.terminal_running = false;
        self.terminal_receiver = None;
        self.error = None;
        self.notice = format!(
            "Workspace ready · {} runner(s) available",
            self.extension_runners.len()
        );
        Ok(())
    }

    fn toggle_directory(&mut self, directory: &Path) {
        let directory = directory.to_path_buf();
        if self.expanded.remove(&directory) {
            return;
        }

        match self.load_directory(&directory) {
            Ok(()) => {
                self.expanded.insert(directory);
            }
            Err(error) => self.show_error(error),
        }
    }

    fn load_directory(&mut self, directory: &Path) -> Result<(), String> {
        let root = self.workspace_root()?;
        let entries = read_directory(&root, directory)?;
        self.directory_cache
            .insert(directory.to_path_buf(), entries);
        Ok(())
    }

    fn open_entry(&mut self, entry: &WorkspaceEntry) {
        if entry.is_directory {
            self.toggle_directory(&entry.path);
            return;
        }

        if let Some(index) = self.tabs.iter().position(|tab| tab.path == entry.path) {
            self.active_tab = Some(index);
            return;
        }

        match read_workspace_file(self.workspace_root(), &entry.path) {
            Ok(file) => {
                self.tabs.push(EditorTab {
                    path: entry.path.clone(),
                    content: file.content.clone(),
                    saved_content: file.content,
                    saved_fingerprint: file.fingerprint,
                });
                self.active_tab = Some(self.tabs.len() - 1);
                self.error = None;
                self.notice = format!("Opened {}", entry.name);
            }
            Err(error) => self.show_error(error),
        }
    }

    fn save_active(&mut self) {
        if let Some(index) = self.active_tab {
            self.save_tab(index);
        }
    }

    fn save_tab(&mut self, index: usize) {
        let Some(tab) = self.tabs.get(index).cloned() else {
            return;
        };
        if !tab.dirty() {
            self.notice = format!("{} is already saved", tab.name());
            return;
        }

        match write_workspace_file(
            self.workspace_root(),
            &tab.path,
            &tab.content,
            tab.saved_fingerprint,
        ) {
            Ok(fingerprint) => {
                if let Some(current) = self.tabs.get_mut(index) {
                    current.saved_content = current.content.clone();
                    current.saved_fingerprint = fingerprint;
                }
                self.error = None;
                self.notice = format!("Saved {}", tab.name());
                self.saved_at = Some(Instant::now());
            }
            Err(error) => self.show_error(error),
        }
    }

    fn save_all(&mut self) {
        let dirty_indices: Vec<usize> = self
            .tabs
            .iter()
            .enumerate()
            .filter_map(|(index, tab)| tab.dirty().then_some(index))
            .collect();
        for index in dirty_indices {
            self.save_tab(index);
        }
        if self.tabs.iter().all(|tab| !tab.dirty()) {
            self.notice = "All files saved".to_owned();
        }
    }

    fn request_close(&mut self, index: usize) {
        if self.tabs.get(index).is_some_and(EditorTab::dirty) {
            self.confirm_close = Some(index);
        } else {
            self.close_tab(index);
        }
    }

    fn close_tab(&mut self, index: usize) {
        if index >= self.tabs.len() {
            return;
        }
        self.tabs.remove(index);
        self.confirm_close = None;
        self.active_tab = match self.active_tab {
            None => None,
            Some(_) if self.tabs.is_empty() => None,
            Some(active) if active > index => Some(active - 1),
            Some(active) if active == index => Some(active.min(self.tabs.len() - 1)),
            Some(active) => Some(active),
        };
    }

    fn replace_matches(&mut self) {
        if self.find_query.is_empty() {
            return;
        }
        let Some(index) = self.active_tab else {
            return;
        };
        if let Some(tab) = self.tabs.get_mut(index) {
            let replaced = tab.content.matches(&self.find_query).count();
            if replaced > 0 {
                tab.content = tab.content.replace(&self.find_query, &self.replace_query);
                self.notice = format!("Replaced {replaced} match(es)");
            } else {
                self.notice = "No matches found".to_owned();
            }
        }
    }

    fn workspace_root(&self) -> Result<PathBuf, String> {
        self.workspace
            .clone()
            .ok_or_else(|| "Open a workspace before accessing files.".to_owned())
    }

    fn show_error(&mut self, error: String) {
        self.error = Some(error.clone());
        self.notice = "Action failed".to_owned();
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        ctx.input_mut(|input| {
            let command = input.modifiers.command;
            if command && input.key_pressed(Key::S) {
                self.save_active();
            }
            if command && input.key_pressed(Key::O) {
                self.open_workspace();
            }
            if command && input.modifiers.shift && input.key_pressed(Key::S) {
                self.save_all();
            }
            if command
                && (input.key_pressed(Key::P)
                    || (input.modifiers.shift && input.key_pressed(Key::P)))
            {
                self.command_palette_open = true;
                self.command_query.clear();
            }
            if command && input.key_pressed(Key::B) {
                self.sidebar_visible = !self.sidebar_visible;
            }
            if command && input.key_pressed(Key::Backtick) {
                self.terminal_visible = !self.terminal_visible;
            }
            if command && input.key_pressed(Key::F) {
                self.find_open = true;
            }
            if input.key_pressed(Key::Escape) {
                self.command_palette_open = false;
                self.find_open = false;
                self.confirm_close = None;
            }
        });
    }

    fn poll_terminal(&mut self) {
        let mut completed = false;
        if let Some(receiver) = &self.terminal_receiver {
            while let Ok(line) = receiver.try_recv() {
                completed = line.starts_with("[completed") || line.starts_with("[failed");
                self.terminal_lines.push(line);
            }
        }
        if completed {
            self.terminal_running = false;
            self.terminal_receiver = None;
        }
    }

    fn active_relative_file(&self) -> Option<PathBuf> {
        let root = self.workspace.as_ref()?;
        let tab = self.tabs.get(self.active_tab?)?;
        tab.path.strip_prefix(root).ok().map(Path::to_path_buf)
    }

    fn runner_available_for_active_file(&self, runner: &ExtensionRunner) -> bool {
        if runner.file_extensions.is_empty() {
            return true;
        }
        self.active_relative_file()
            .and_then(|path| {
                path.extension()
                    .and_then(|extension| extension.to_str())
                    .map(str::to_owned)
            })
            .is_some_and(|extension| {
                runner
                    .file_extensions
                    .iter()
                    .any(|expected| expected.eq_ignore_ascii_case(&extension))
            })
    }

    fn run_extension(&mut self, id: &str) {
        let Some(runner) = self
            .extension_runners
            .iter()
            .find(|runner| runner.id == id)
            .cloned()
        else {
            return;
        };
        if !self.runner_available_for_active_file(&runner) {
            self.show_error(format!(
                "{} needs an active {} file.",
                runner.name,
                runner.file_extensions.join(" or .")
            ));
            return;
        }
        let file = self.active_relative_file();
        if !runner.file_extensions.is_empty() && file.is_none() {
            self.show_error("Open a file before running this extension.".to_owned());
            return;
        }
        let args = expand_runner_args(&runner.args, file.as_deref());
        self.run_process(&runner.name, runner.program, args);
    }

    fn run_terminal_input(&mut self) {
        let command = self.terminal_input.trim().to_owned();
        if command.is_empty() || self.terminal_running {
            return;
        }
        self.terminal_input.clear();
        #[cfg(windows)]
        self.run_process("Terminal", "cmd".to_owned(), vec!["/C".to_owned(), command]);
        #[cfg(not(windows))]
        self.run_process("Terminal", "sh".to_owned(), vec!["-lc".to_owned(), command]);
    }

    fn run_process(&mut self, label: &str, program: String, args: Vec<String>) {
        if self.terminal_running {
            self.notice = "A terminal command is already running".to_owned();
            return;
        }
        let Ok(root) = self.workspace_root() else {
            self.show_error("Open a workspace before running a command.".to_owned());
            return;
        };
        let display = std::iter::once(program.clone())
            .chain(args.iter().cloned())
            .collect::<Vec<_>>()
            .join(" ");
        self.terminal_lines.push(format!("ns > {display}"));
        self.terminal_visible = true;
        self.terminal_running = true;
        self.notice = format!("Running {label}");
        let (sender, receiver) = mpsc::channel();
        let label = label.to_owned();
        thread::spawn(move || {
            let result = Command::new(&program)
                .args(&args)
                .current_dir(root)
                .output();
            match result {
                Ok(output) => {
                    let stdout = String::from_utf8_lossy(&output.stdout).trim().to_owned();
                    let stderr = String::from_utf8_lossy(&output.stderr).trim().to_owned();
                    if !stdout.is_empty() {
                        let _ = sender.send(stdout);
                    }
                    if !stderr.is_empty() {
                        let _ = sender.send(stderr);
                    }
                    let status = if output.status.success() {
                        "completed"
                    } else {
                        "failed"
                    };
                    let _ = sender.send(format!("[{status}: {label} ({})]", output.status));
                }
                Err(error) => {
                    let _ = sender.send(format!("[failed: could not start {label}: {error}]"));
                }
            }
        });
        self.terminal_receiver = Some(receiver);
    }

    fn draw_top_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("top_bar")
            .frame(
                egui::Frame::NONE
                    .fill(PANEL_BG)
                    .stroke(egui::Stroke::new(1.0_f32, BORDER)),
            )
            .show(ctx, |ui| {
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.add_space(7.0);
                    draw_logo(ui, 27.0);
                    ui.label(RichText::new("NS Environ").strong().color(TEXT));
                    ui.label(
                        RichText::new(
                            self.workspace
                                .as_deref()
                                .and_then(Path::file_name)
                                .and_then(|name| name.to_str())
                                .unwrap_or("No project open"),
                        )
                        .small()
                        .color(MUTED),
                    );
                    ui.add_space(64.0);
                    if ui
                        .add_sized(
                            [390.0, 29.0],
                            egui::Button::new(
                                RichText::new(
                                    "⌕   Search files and commands                       Ctrl/⌘ P",
                                )
                                .small()
                                .color(MUTED),
                            )
                            .fill(TERMINAL_BG)
                            .stroke(egui::Stroke::new(1.0_f32, BORDER)),
                        )
                        .clicked()
                    {
                        self.command_palette_open = true;
                        self.command_query.clear();
                    }
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui.button("Open folder").clicked() {
                            self.open_workspace();
                        }
                        ui.label(RichText::new("◧   ▱   ◐").color(MUTED));
                    });
                });
                ui.add_space(4.0);
            });
    }

    fn draw_activity_bar(&mut self, ctx: &egui::Context) {
        egui::SidePanel::left("activity_bar")
            .exact_width(44.0)
            .resizable(false)
            .frame(
                egui::Frame::NONE
                    .fill(TERMINAL_BG)
                    .stroke(egui::Stroke::new(1.0_f32, BORDER)),
            )
            .show(ctx, |ui| {
                ui.add_space(5.0);
                for (view, icon, label) in [
                    (SidebarView::Explorer, "▤", "Explorer"),
                    (SidebarView::Search, "⌕", "Search workspace"),
                    (SidebarView::Extensions, "▦", "Extensions"),
                ] {
                    let active = self.sidebar_visible && self.sidebar_view == view;
                    let button =
                        egui::Button::new(RichText::new(icon).size(21.0).color(if active {
                            TEXT
                        } else {
                            MUTED
                        }))
                        .fill(if active { ACTIVE } else { Color32::TRANSPARENT })
                        .frame(false);
                    if ui
                        .add_sized([34.0, 37.0], button)
                        .on_hover_text(label)
                        .clicked()
                    {
                        if active {
                            self.sidebar_visible = false;
                        } else {
                            self.sidebar_visible = true;
                            self.sidebar_view = view;
                        }
                    }
                }
                ui.with_layout(Layout::bottom_up(Align::Center), |ui| {
                    if ui
                        .add_sized(
                            [34.0, 34.0],
                            egui::Button::new(RichText::new("⚙").color(MUTED)).frame(false),
                        )
                        .on_hover_text("Manage")
                        .clicked()
                    {
                        self.about_open = true;
                    }
                });
            });
    }

    fn draw_sidebar(&mut self, ctx: &egui::Context) {
        if !self.sidebar_visible {
            return;
        }
        egui::SidePanel::left("sidebar")
            .default_width(270.0)
            .resizable(true)
            .frame(
                egui::Frame::NONE
                    .fill(PANEL_BG)
                    .stroke(egui::Stroke::new(1.0_f32, BORDER)),
            )
            .show(ctx, |ui| match self.sidebar_view {
                SidebarView::Explorer => self.draw_explorer_contents(ui),
                SidebarView::Search => self.draw_search_contents(ui),
                SidebarView::Extensions => self.draw_extension_contents(ui),
            });
    }

    fn draw_explorer_contents(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("EXPLORER").small().strong().color(MUTED));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("+").on_hover_text("Open folder").clicked() {
                    self.open_workspace();
                }
            });
        });
        ui.separator();

        let Some(root) = self.workspace.clone() else {
            ui.vertical_centered(|ui| {
                ui.add_space(120.0);
                draw_logo(ui, 48.0);
                ui.add_space(8.0);
                ui.label(RichText::new("Your project files").color(MUTED));
                ui.label(RichText::new("will appear here.").color(MUTED));
                if ui.button("Open a folder").clicked() {
                    self.open_workspace();
                }
            });
            return;
        };

        egui::ScrollArea::vertical().show(ui, |ui| {
            self.draw_tree(ui, &root, 0);
        });
    }

    fn draw_search_contents(&mut self, ui: &mut egui::Ui) {
        ui.label(RichText::new("SEARCH").small().strong().color(MUTED));
        ui.add_space(5.0);
        ui.add(TextEdit::singleline(&mut self.search_query).hint_text("Search open files"));
        ui.separator();
        let query = self.search_query.to_lowercase();
        let matches: Vec<(usize, String)> = self
            .tabs
            .iter()
            .enumerate()
            .filter_map(|(index, tab)| {
                let name = tab.name();
                (query.is_empty()
                    || name.to_lowercase().contains(&query)
                    || tab.content.to_lowercase().contains(&query))
                .then_some((index, name))
            })
            .collect();
        if matches.is_empty() {
            ui.label(
                RichText::new("Open files are searchable here.")
                    .small()
                    .color(MUTED),
            );
        }
        for (index, name) in matches {
            if ui
                .selectable_label(self.active_tab == Some(index), format!("◇  {name}"))
                .clicked()
            {
                self.active_tab = Some(index);
            }
        }
    }

    fn draw_extension_contents(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("EXTENSIONS").small().strong().color(MUTED));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui
                    .small_button("↻")
                    .on_hover_text("Reload environ.extensions.json")
                    .clicked()
                {
                    self.extension_runners = load_extensions(self.workspace.as_deref());
                    self.notice = format!("Loaded {} runner(s)", self.extension_runners.len());
                }
            });
        });
        ui.separator();
        ui.label(RichText::new("Runnable extensions").small().color(MUTED));
        ui.add_space(4.0);
        let runners = self.extension_runners.clone();
        for runner in runners {
            ui.group(|ui| {
                ui.label(RichText::new(&runner.name).strong().color(TEXT));
                ui.label(RichText::new(&runner.description).small().color(MUTED));
                ui.horizontal(|ui| {
                    let enabled =
                        !self.terminal_running && self.runner_available_for_active_file(&runner);
                    if ui.add_enabled(enabled, egui::Button::new("Run")).clicked() {
                        self.run_extension(&runner.id);
                    }
                    if !runner.file_extensions.is_empty() {
                        ui.label(
                            RichText::new(format!(".{}", runner.file_extensions.join(", .")))
                                .small()
                                .color(MUTED),
                        );
                    }
                });
            });
            ui.add_space(4.0);
        }
        ui.separator();
        ui.label(RichText::new("Add project runners in environ.extensions.json. Each runner is shown here and only runs when you click Run.").small().color(MUTED));
    }

    fn draw_tree(&mut self, ui: &mut egui::Ui, directory: &Path, depth: usize) {
        let Some(entries) = self.directory_cache.get(directory).cloned() else {
            return;
        };
        for entry in entries {
            ui.horizontal(|ui| {
                ui.add_space(depth as f32 * 14.0);
                let label = if entry.is_directory {
                    let marker = if self.expanded.contains(&entry.path) {
                        "⌄"
                    } else {
                        "›"
                    };
                    format!("{marker}  📁  {}", entry.name)
                } else {
                    format!("   ◇  {}", entry.name)
                };
                let response =
                    ui.selectable_label(false, RichText::new(label).color(TEXT).size(12.0));
                if response.clicked() {
                    self.open_entry(&entry);
                }
                if response.hovered() {
                    response.on_hover_text(entry.path.display().to_string());
                }
            });
            if entry.is_directory && self.expanded.contains(&entry.path) {
                self.draw_tree(ui, &entry.path, depth + 1);
            }
        }
    }

    fn draw_editor(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(EDITOR_BG))
            .show(ctx, |ui| {
                if self.tabs.is_empty() {
                    self.draw_welcome(ui);
                    return;
                }

                ui.horizontal(|ui| {
                    for index in 0..self.tabs.len() {
                        let active = self.active_tab == Some(index);
                        let tab = &self.tabs[index];
                        let mut label = tab.name();
                        if tab.dirty() {
                            label.push_str(" •");
                        }
                        let response = ui.add(
                            egui::Button::new(RichText::new(label).color(if active {
                                TEXT
                            } else {
                                MUTED
                            }))
                            .fill(if active {
                                PANEL_BG
                            } else {
                                Color32::TRANSPARENT
                            })
                            .stroke(egui::Stroke::new(
                                if active { 1.0_f32 } else { 0.0_f32 },
                                if active { ACCENT } else { Color32::TRANSPARENT },
                            )),
                        );
                        if response.clicked() {
                            self.active_tab = Some(index);
                        }
                        if ui.small_button("×").on_hover_text("Close tab").clicked() {
                            self.request_close(index);
                            break;
                        }
                    }
                });
                ui.separator();

                if let Some(index) = self.active_tab
                    && let Some(tab) = self.tabs.get(index)
                    && let Some(root) = &self.workspace
                {
                    let path = tab.path.strip_prefix(root).unwrap_or(&tab.path);
                    ui.label(
                        RichText::new(format!("{}", path.display()))
                            .small()
                            .color(MUTED),
                    );
                    ui.separator();
                }

                if self.find_open {
                    ui.horizontal(|ui| {
                        ui.label("Find");
                        ui.add(TextEdit::singleline(&mut self.find_query).desired_width(180.0));
                        ui.label(format!("{} match(es)", self.match_count()));
                        ui.label("Replace");
                        ui.add(TextEdit::singleline(&mut self.replace_query).desired_width(180.0));
                        if ui.button("Replace all").clicked() {
                            self.replace_matches();
                        }
                        if ui.button("×").clicked() {
                            self.find_open = false;
                        }
                    });
                    ui.separator();
                }

                let Some(index) = self.active_tab else {
                    return;
                };
                if let Some(tab) = self.tabs.get_mut(index) {
                    let editor = TextEdit::multiline(&mut tab.content)
                        .font(FontId::monospace(14.0))
                        .desired_width(f32::INFINITY)
                        .desired_rows(30)
                        .lock_focus(true)
                        .hint_text("Start writing…");
                    ui.add_sized(ui.available_size(), editor);
                }
            });
    }

    fn draw_welcome(&mut self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            ui.add_space(ui.available_height() * 0.22);
            draw_logo(ui, 76.0);
            ui.add_space(18.0);
            ui.label(
                RichText::new("A quiet place to write code")
                    .small()
                    .color(ACCENT),
            );
            ui.add_space(8.0);
            ui.heading(RichText::new("NS Environ").color(TEXT).size(27.0));
            ui.label(
                RichText::new("A VS Code-style workspace, built independently in Rust.")
                    .color(MUTED),
            );
            ui.add_space(18.0);
            if ui.button("Open project folder  ↗").clicked() {
                self.open_workspace();
            }
            ui.add_space(10.0);
            ui.label(
                RichText::new("Ctrl/⌘ P command search  ·  Ctrl/⌘ B sidebar  ·  Ctrl/⌘ ` terminal")
                    .small()
                    .color(MUTED),
            );
        });
    }

    fn draw_status_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("status_bar")
            .frame(egui::Frame::NONE.fill(Color32::from_rgb(32, 43, 92)))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("ns  ›").strong().color(TEXT));
                    ui.label(
                        RichText::new(self.notice.clone())
                            .small()
                            .color(Color32::from_rgb(211, 219, 255)),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if let Some(error) = &self.error {
                            ui.label(
                                RichText::new(error)
                                    .small()
                                    .color(Color32::from_rgb(237, 155, 145)),
                            );
                        }
                        ui.label(
                            RichText::new("UTF-8")
                                .small()
                                .color(Color32::from_rgb(211, 219, 255)),
                        );
                        ui.label(
                            RichText::new("Spaces: 2")
                                .small()
                                .color(Color32::from_rgb(211, 219, 255)),
                        );
                        ui.label(
                            RichText::new("0 problems")
                                .small()
                                .color(Color32::from_rgb(211, 219, 255)),
                        );
                    });
                });
            });
    }

    fn draw_terminal(&mut self, ctx: &egui::Context) {
        if !self.terminal_visible {
            return;
        }
        egui::TopBottomPanel::bottom("terminal")
            .default_height(210.0)
            .min_height(120.0)
            .resizable(true)
            .frame(
                egui::Frame::NONE
                    .fill(TERMINAL_BG)
                    .stroke(egui::Stroke::new(1.0_f32, BORDER)),
            )
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("TERMINAL").small().strong().color(TEXT));
                    ui.label(
                        RichText::new(if self.terminal_running {
                            "Running…"
                        } else {
                            "Problems 0"
                        })
                        .small()
                        .color(MUTED),
                    );
                    ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                        if ui
                            .small_button("×")
                            .on_hover_text("Hide terminal (Ctrl/⌘ `)")
                            .clicked()
                        {
                            self.terminal_visible = false;
                        }
                        if ui.small_button("Clear").clicked() && !self.terminal_running {
                            self.terminal_lines.clear();
                        }
                    });
                });
                ui.separator();
                egui::ScrollArea::vertical()
                    .stick_to_bottom(true)
                    .show(ui, |ui| {
                        for line in &self.terminal_lines {
                            ui.label(
                                RichText::new(line)
                                    .font(FontId::monospace(12.0))
                                    .color(TEXT),
                            );
                        }
                    });
                ui.horizontal(|ui| {
                    ui.label(RichText::new("ns ›").monospace().color(ACCENT));
                    let response = ui.add_enabled(
                        !self.terminal_running,
                        TextEdit::singleline(&mut self.terminal_input)
                            .font(FontId::monospace(13.0))
                            .desired_width(f32::INFINITY)
                            .hint_text("Type a command and press Enter"),
                    );
                    if response.lost_focus() && ui.input(|input| input.key_pressed(Key::Enter)) {
                        self.run_terminal_input();
                    }
                });
            });
    }

    fn draw_command_palette(&mut self, ctx: &egui::Context) {
        if !self.command_palette_open {
            return;
        }
        let mut close = false;
        egui::Window::new("Command Palette")
            .collapsible(false)
            .resizable(false)
            .default_width(460.0)
            .show(ctx, |ui| {
                ui.add(TextEdit::singleline(&mut self.command_query).hint_text("Type a command…"));
                ui.add_space(6.0);
                let query = self.command_query.to_lowercase();
                let commands = [
                    ("Open folder", "Choose a workspace folder"),
                    ("Save file", "Save the active editor tab"),
                    ("Save all files", "Save every dirty tab"),
                    ("Find in file", "Open find and replace"),
                    ("Toggle sidebar", "Show or hide the workspace sidebar"),
                    ("Toggle terminal", "Show or hide the integrated terminal"),
                    ("Show extensions", "Open runnable extensions"),
                    ("Run Rust workspace", "Run cargo run in this workspace"),
                    ("Run active Python file", "Run the active .py file"),
                    ("Run active Node file", "Run the active JavaScript file"),
                    ("About Environ", "View version and project information"),
                ];
                for (name, description) in commands {
                    if !query.is_empty() && !name.to_lowercase().contains(&query) {
                        continue;
                    }
                    if ui
                        .selectable_label(false, format!("{name}  —  {description}"))
                        .clicked()
                    {
                        match name {
                            "Open folder" => self.open_workspace(),
                            "Save file" => self.save_active(),
                            "Save all files" => self.save_all(),
                            "Find in file" => self.find_open = true,
                            "Toggle sidebar" => self.sidebar_visible = !self.sidebar_visible,
                            "Toggle terminal" => self.terminal_visible = !self.terminal_visible,
                            "Show extensions" => {
                                self.sidebar_visible = true;
                                self.sidebar_view = SidebarView::Extensions;
                            }
                            "Run Rust workspace" => self.run_extension("rust-workspace"),
                            "Run active Python file" => self.run_extension("python-file"),
                            "Run active Node file" => self.run_extension("node-file"),
                            "About Environ" => self.about_open = true,
                            _ => {}
                        }
                        close = true;
                    }
                }
            });
        if close {
            self.command_palette_open = false;
        }
    }

    fn draw_close_confirmation(&mut self, ctx: &egui::Context) {
        let Some(index) = self.confirm_close else {
            return;
        };
        let name = self
            .tabs
            .get(index)
            .map(EditorTab::name)
            .unwrap_or_else(|| "file".to_owned());
        let mut close = false;
        egui::Window::new("Unsaved changes")
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.label(format!("Save changes to {name} before closing?"));
                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() {
                        self.save_tab(index);
                        close = true;
                    }
                    if ui.button("Discard").clicked() {
                        self.close_tab(index);
                    }
                    if ui.button("Cancel").clicked() {
                        close = true;
                    }
                });
            });
        if close {
            self.confirm_close = None;
        }
    }

    fn draw_about(&mut self, ctx: &egui::Context) {
        if !self.about_open {
            return;
        }
        egui::Window::new("About Environ")
            .collapsible(false)
            .resizable(false)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    draw_logo(ui, 45.0);
                    ui.vertical(|ui| {
                        ui.label(RichText::new("Environ").heading().color(TEXT));
                        ui.label(RichText::new("Rust-native focused code editor").color(MUTED));
                    });
                });
                ui.separator();
                ui.label("A lightweight, VS Code-inspired workbench for focused project work.");
                ui.label("Built with Rust and egui.");
                if ui.button("Close").clicked() {
                    self.about_open = false;
                }
            });
    }

    fn match_count(&self) -> usize {
        let Some(index) = self.active_tab else {
            return 0;
        };
        if self.find_query.is_empty() {
            return 0;
        }
        self.tabs
            .get(index)
            .map(|tab| tab.content.matches(&self.find_query).count())
            .unwrap_or(0)
    }
}

impl App for EnvironApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut Frame) {
        self.poll_terminal();
        self.handle_shortcuts(ctx);
        self.draw_top_bar(ctx);
        self.draw_activity_bar(ctx);
        self.draw_sidebar(ctx);
        self.draw_status_bar(ctx);
        self.draw_terminal(ctx);
        self.draw_editor(ctx);
        self.draw_command_palette(ctx);
        self.draw_close_confirmation(ctx);
        self.draw_about(ctx);
        if self
            .saved_at
            .is_some_and(|saved| saved.elapsed().as_secs() < 2)
            || self.terminal_running
        {
            ctx.request_repaint_after(std::time::Duration::from_millis(250));
        }
    }
}

fn builtin_extensions() -> Vec<ExtensionRunner> {
    vec![
        ExtensionRunner {
            id: "rust-workspace".to_owned(),
            name: "Rust workspace runner".to_owned(),
            description: "Runs cargo run at the workspace root.".to_owned(),
            program: "cargo".to_owned(),
            args: vec!["run".to_owned()],
            file_extensions: Vec::new(),
        },
        ExtensionRunner {
            id: "python-file".to_owned(),
            name: "Python runner".to_owned(),
            description: "Runs the active Python source file.".to_owned(),
            program: "python".to_owned(),
            args: vec!["{file}".to_owned()],
            file_extensions: vec!["py".to_owned()],
        },
        ExtensionRunner {
            id: "node-file".to_owned(),
            name: "Node.js runner".to_owned(),
            description: "Runs the active JavaScript file with Node.js.".to_owned(),
            program: "node".to_owned(),
            args: vec!["{file}".to_owned()],
            file_extensions: vec!["js".to_owned(), "mjs".to_owned(), "cjs".to_owned()],
        },
    ]
}

fn load_extensions(workspace: Option<&Path>) -> Vec<ExtensionRunner> {
    let mut runners = builtin_extensions();
    let Some(root) = workspace else {
        return runners;
    };
    let manifest_path = root.join("environ.extensions.json");
    let Ok(contents) = fs::read_to_string(manifest_path) else {
        return runners;
    };
    let Ok(manifest) = serde_json::from_str::<ExtensionManifest>(&contents) else {
        return runners;
    };
    runners.extend(manifest.extensions.into_iter().filter_map(|extension| {
        let valid_id = !extension.id.trim().is_empty();
        let valid_program = !extension.program.trim().is_empty()
            && !Path::new(&extension.program).is_absolute()
            && !extension.program.contains(['/', '\\']);
        (valid_id && valid_program).then_some(ExtensionRunner {
            id: extension.id,
            name: extension.name,
            description: extension.description,
            program: extension.program,
            args: extension.args,
            file_extensions: extension.file_extensions,
        })
    }));
    runners
}

fn expand_runner_args(args: &[String], file: Option<&Path>) -> Vec<String> {
    let file = file
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_default();
    args.iter()
        .map(|argument| argument.replace("{file}", &file))
        .collect()
}

fn draw_logo(ui: &mut egui::Ui, size: f32) {
    let (rect, _) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::hover());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, size * 0.27, ACCENT);
    painter.text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        "NS",
        FontId::proportional(size * 0.68),
        Color32::from_rgb(14, 16, 20),
    );
}

fn read_directory(root: &Path, directory: &Path) -> Result<Vec<WorkspaceEntry>, String> {
    let canonical_root =
        fs::canonicalize(root).map_err(|error| format!("Could not access workspace: {error}"))?;
    let canonical_directory =
        fs::canonicalize(directory).map_err(|error| format!("Could not access folder: {error}"))?;
    ensure_within(&canonical_root, &canonical_directory)?;

    let mut entries = Vec::new();
    let iterator = fs::read_dir(&canonical_directory)
        .map_err(|error| format!("Could not read folder: {error}"))?;
    for entry in iterator {
        let entry = entry.map_err(|error| format!("Could not inspect folder entry: {error}"))?;
        let path = entry.path();
        let Ok(canonical_path) = fs::canonicalize(&path) else {
            continue;
        };
        if ensure_within(&canonical_root, &canonical_path).is_err() {
            continue;
        }
        let metadata = match fs::metadata(&canonical_path) {
            Ok(metadata) => metadata,
            Err(_) => continue,
        };
        if !metadata.is_dir() && !metadata.is_file() {
            continue;
        }
        entries.push(WorkspaceEntry {
            path: canonical_path,
            name: entry.file_name().to_string_lossy().into_owned(),
            is_directory: metadata.is_dir(),
        });
    }
    entries.sort_by(|left, right| {
        right
            .is_directory
            .cmp(&left.is_directory)
            .then_with(|| left.name.to_lowercase().cmp(&right.name.to_lowercase()))
    });
    Ok(entries)
}

fn read_workspace_file(root: Result<PathBuf, String>, path: &Path) -> Result<LoadedFile, String> {
    let root = root?;
    let canonical_root =
        fs::canonicalize(&root).map_err(|error| format!("Could not access workspace: {error}"))?;
    let canonical_file =
        fs::canonicalize(path).map_err(|error| format!("Could not open file: {error}"))?;
    ensure_within(&canonical_root, &canonical_file)?;
    let metadata = fs::metadata(&canonical_file)
        .map_err(|error| format!("Could not inspect file: {error}"))?;
    if !metadata.is_file() {
        return Err("The requested path is not a file.".to_owned());
    }
    if metadata.len() > MAX_FILE_BYTES {
        return Err("This file is larger than Environ's 2 MB editor limit.".to_owned());
    }
    let bytes =
        fs::read(&canonical_file).map_err(|error| format!("Could not read file: {error}"))?;
    if bytes.contains(&0) {
        return Err("Binary files cannot be opened in the text editor.".to_owned());
    }
    let fingerprint = fingerprint(&bytes);
    let content =
        String::from_utf8(bytes).map_err(|_| "This file is not valid UTF-8 text.".to_owned())?;
    Ok(LoadedFile {
        content,
        fingerprint,
    })
}

fn write_workspace_file(
    root: Result<PathBuf, String>,
    path: &Path,
    content: &str,
    expected_fingerprint: u64,
) -> Result<u64, String> {
    let root = root?;
    let canonical_root =
        fs::canonicalize(&root).map_err(|error| format!("Could not access workspace: {error}"))?;
    let canonical_file =
        fs::canonicalize(path).map_err(|error| format!("Could not access file: {error}"))?;
    ensure_within(&canonical_root, &canonical_file)?;
    let metadata = fs::metadata(&canonical_file)
        .map_err(|error| format!("Could not inspect file: {error}"))?;
    if !metadata.is_file() {
        return Err("The requested path is not a file.".to_owned());
    }
    if metadata.len() > MAX_FILE_BYTES {
        return Err(
            "The file changed on disk and is now larger than Environ's 2 MB editor limit."
                .to_owned(),
        );
    }
    let current_bytes = fs::read(&canonical_file)
        .map_err(|error| format!("Could not verify the current file before saving: {error}"))?;
    if fingerprint(&current_bytes) != expected_fingerprint {
        return Err(
            "The file changed on disk since it was opened. Reload it or resolve the conflict before saving."
                .to_owned(),
        );
    }

    let unique = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let temporary = canonical_file.with_file_name(format!(
        ".{}.environ-tmp-{}-{}",
        canonical_file
            .file_name()
            .and_then(|name| name.to_str())
            .unwrap_or("file"),
        std::process::id(),
        unique
    ));
    let mut temporary_file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temporary)
        .map_err(|error| format!("Could not create a temporary save file: {error}"))?;
    if let Err(error) = temporary_file
        .write_all(content.as_bytes())
        .and_then(|()| temporary_file.sync_all())
    {
        let _ = fs::remove_file(&temporary);
        return Err(format!("Could not flush the saved file: {error}"));
    }
    drop(temporary_file);
    if let Err(error) = atomic_replace(&temporary, &canonical_file) {
        let _ = fs::remove_file(&temporary);
        return Err(format!("Could not replace the saved file: {error}"));
    }
    Ok(fingerprint(content.as_bytes()))
}

fn fingerprint(bytes: &[u8]) -> u64 {
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    bytes.hash(&mut hasher);
    hasher.finish()
}

#[cfg(not(windows))]
fn atomic_replace(source: &Path, destination: &Path) -> io::Result<()> {
    fs::rename(source, destination)
}

#[cfg(windows)]
fn atomic_replace(source: &Path, destination: &Path) -> io::Result<()> {
    use std::os::windows::ffi::OsStrExt;

    unsafe extern "system" {
        fn MoveFileExW(
            existing_file_name: *const u16,
            new_file_name: *const u16,
            flags: u32,
        ) -> i32;
    }

    const MOVEFILE_REPLACE_EXISTING: u32 = 0x1;
    const MOVEFILE_WRITE_THROUGH: u32 = 0x8;
    let source_wide: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
    let destination_wide: Vec<u16> = destination
        .as_os_str()
        .encode_wide()
        .chain(Some(0))
        .collect();
    let result = unsafe {
        MoveFileExW(
            source_wide.as_ptr(),
            destination_wide.as_ptr(),
            MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH,
        )
    };
    if result == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn ensure_within(root: &Path, candidate: &Path) -> Result<(), String> {
    if candidate == root || candidate.starts_with(root) {
        Ok(())
    } else {
        Err("The requested path is outside the open workspace.".to_owned())
    }
}

fn logo_icon() -> egui::IconData {
    let size = 64u32;
    let mut rgba = vec![0u8; (size * size * 4) as usize];
    for y in 0..size {
        for x in 0..size {
            let dx = x as f32 - 32.0;
            let dy = y as f32 - 32.0;
            if dx * dx + dy * dy < 29.0 * 29.0 {
                let index = ((y * size + x) * 4) as usize;
                rgba[index..index + 4].copy_from_slice(&[145, 198, 174, 255]);
            }
        }
    }
    egui::IconData {
        rgba,
        width: size,
        height: size,
    }
}

fn main() -> eframe::Result {
    let options = NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([960.0, 620.0])
            .with_icon(logo_icon()),
        ..Default::default()
    };
    eframe::run_native(
        "Environ",
        options,
        Box::new(|cc| Ok(Box::new(EnvironApp::new(cc)))),
    )
}

#[cfg(test)]
mod tests {
    use super::{ensure_within, expand_runner_args, fingerprint};
    use std::path::{Path, PathBuf};

    #[test]
    fn fingerprints_change_when_file_bytes_change() {
        assert_ne!(fingerprint(b"before"), fingerprint(b"after"));
        assert_eq!(fingerprint(b"same"), fingerprint(b"same"));
    }

    #[test]
    fn workspace_containment_accepts_root_and_children() {
        let root = Path::new("C:/workspace");
        assert!(ensure_within(root, root).is_ok());
        assert!(ensure_within(root, Path::new("C:/workspace/src/main.rs")).is_ok());
        assert!(ensure_within(root, Path::new("C:/other/main.rs")).is_err());
    }

    #[test]
    fn runner_arguments_expand_the_active_workspace_file() {
        let file = PathBuf::from("src/main.py");
        assert_eq!(
            expand_runner_args(&["{file}".to_owned(), "--watch".to_owned()], Some(&file)),
            vec!["src/main.py", "--watch"]
        );
    }
}
