use eframe::egui;
use crate::sip::ua::{RegistrationState, Call};
use std::sync::{Arc, Mutex};

pub enum UiCommand {
    Register,
    Invite(String),
    Hangup(String),
}

pub struct SoftphoneApp {
    pub dialer_input: String,
    pub call_history: Vec<String>,
    pub reg_state: Arc<Mutex<RegistrationState>>,
    pub active_calls: Arc<Mutex<Vec<Call>>>,
    pub command_sender: tokio::sync::mpsc::UnboundedSender<UiCommand>,
}

impl eframe::App for SoftphoneApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading("Softphone");

            ui.separator();

            self.render_account_status(ui);
            ui.separator();

            self.render_dialer(ui);
            ui.separator();

            self.render_active_calls(ui);
            ui.separator();

            self.render_call_history(ui);
        });

        // Request a repaint to keep the UI updated with the state from background tasks
        ctx.request_repaint();
    }
}

impl SoftphoneApp {
    pub fn new(
        command_sender: tokio::sync::mpsc::UnboundedSender<UiCommand>,
        reg_state: Arc<Mutex<RegistrationState>>,
        active_calls: Arc<Mutex<Vec<Call>>>,
    ) -> Self {
        Self {
            dialer_input: String::new(),
            call_history: Vec::new(),
            reg_state,
            active_calls,
            command_sender,
        }
    }

    fn render_account_status(&mut self, ui: &mut egui::Ui) {
        let state = self.reg_state.lock().unwrap().clone();
        ui.horizontal(|ui| {
            ui.label("Status: ");
            match state {
                RegistrationState::Unregistered => {
                    ui.label(egui::RichText::new("Unregistered").color(egui::Color32::GRAY));
                }
                RegistrationState::Registering => {
                    ui.label(egui::RichText::new("Registering...").color(egui::Color32::YELLOW));
                }
                RegistrationState::Registered => {
                    ui.label(egui::RichText::new("Registered").color(egui::Color32::GREEN));
                }
                RegistrationState::Failed(err) => {
                    ui.label(egui::RichText::new(format!("Failed: {}", err)).color(egui::Color32::RED));
                }
            }
            if ui.button("Register").clicked() {
                let _ = self.command_sender.send(UiCommand::Register);
            }
        });
    }

    fn render_dialer(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.label("Number/URI: ");
            ui.text_edit_singleline(&mut self.dialer_input);
            if ui.button("Call").clicked() {
                if !self.dialer_input.is_empty() {
                    let destination = self.dialer_input.clone();
                    self.call_history.push(destination.clone());
                    let _ = self.command_sender.send(UiCommand::Invite(destination));
                }
            }
        });
    }

    fn render_active_calls(&mut self, ui: &mut egui::Ui) {
        ui.label("Active Calls:");
        let calls = self.active_calls.lock().unwrap().clone();
        if calls.is_empty() {
            ui.label("No active calls");
        } else {
            for call in calls {
                ui.horizontal(|ui| {
                    ui.label(format!("{} [{:?}]", call.remote_uri, call.state));
                    if ui.button("Hangup").clicked() {
                        let _ = self.command_sender.send(UiCommand::Hangup(call.id));
                    }
                });
            }
        }
    }

    fn render_call_history(&mut self, ui: &mut egui::Ui) {
        ui.collapsing("Call History", |ui| {
            for entry in &self.call_history {
                ui.label(entry);
            }
        });
    }
}
