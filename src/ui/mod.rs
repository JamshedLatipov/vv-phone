use eframe::egui;
use crate::sip::ua::{RegistrationState, Call};
use std::sync::{Arc, Mutex};
use crate::core::Account;
use crate::config::{Config, TransportType, ConnectionSettings};

pub enum UiCommand {
    Register(Account),
    Invite(String),
    Hangup(String),
    SaveConfig(Config),
}

pub struct SoftphoneApp {
    pub account_name: String,
    pub account_username: String,
    pub account_domain: String,
    pub account_password: String,
    pub bind_address: String,
    pub transport_type: TransportType,
    pub dialer_input: String,
    pub call_history: Vec<String>,
    pub reg_state: Arc<Mutex<RegistrationState>>,
    pub active_calls: Arc<Mutex<Vec<Call>>>,
    pub command_sender: tokio::sync::mpsc::UnboundedSender<UiCommand>,
}

impl eframe::App for SoftphoneApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        // Apply modern dark theme
        ctx.set_visuals(egui::Visuals::dark());

        egui::SidePanel::left("settings_panel")
            .resizable(false)
            .default_width(250.0)
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.add_space(10.0);
                    ui.heading(egui::RichText::new("⚙ Settings").strong());
                    ui.add_space(10.0);
                });

                ui.separator();
                ui.add_space(10.0);

                ui.label(egui::RichText::new("Account").strong());
                ui.add_space(5.0);

                ui.label("Account Name");
                ui.text_edit_singleline(&mut self.account_name);
                ui.add_space(5.0);

                ui.label("Username");
                ui.text_edit_singleline(&mut self.account_username);
                ui.add_space(5.0);

                ui.label("Domain / Proxy");
                ui.text_edit_singleline(&mut self.account_domain);
                ui.add_space(5.0);

                ui.label("Password");
                ui.add(egui::TextEdit::singleline(&mut self.account_password).password(true));
                ui.add_space(10.0);

                ui.separator();
                ui.add_space(10.0);
                ui.label(egui::RichText::new("Connection").strong());
                ui.add_space(5.0);

                ui.label("Bind Address");
                ui.text_edit_singleline(&mut self.bind_address);
                ui.add_space(5.0);

                ui.label("Transport");
                egui::ComboBox::from_label("")
                    .selected_text(format!("{:?}", self.transport_type))
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut self.transport_type, TransportType::Udp, "UDP");
                        ui.selectable_value(&mut self.transport_type, TransportType::Tcp, "TCP");
                    });

                ui.add_space(15.0);

                if ui.add(egui::Button::new(egui::RichText::new("Save & Register").strong())
                    .min_size(egui::vec2(ui.available_width(), 30.0)))
                    .clicked()
                {
                    let account = Account {
                        name: self.account_name.clone(),
                        username: self.account_username.clone(),
                        domain: self.account_domain.clone(),
                        password: Some(self.account_password.clone()),
                        proxy: None,
                    };

                    let config = Config {
                        accounts: vec![account.clone()],
                        connection: ConnectionSettings {
                            bind_address: self.bind_address.clone(),
                            transport_type: self.transport_type.clone(),
                        },
                    };

                    let _ = self.command_sender.send(UiCommand::SaveConfig(config));
                    let _ = self.command_sender.send(UiCommand::Register(account));
                }

                ui.add_space(20.0);
                ui.separator();
                self.render_call_history(ui);
            });

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.heading(egui::RichText::new("📞 Softphone").strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    self.render_status_badge(ui);
                });
            });
            ui.add_space(10.0);
            ui.separator();
            ui.add_space(20.0);

            self.render_dialer(ui);
            ui.add_space(20.0);

            self.render_active_calls(ui);
        });

        ctx.request_repaint();
    }
}

impl SoftphoneApp {
    pub fn new(
        initial_config: Config,
        command_sender: tokio::sync::mpsc::UnboundedSender<UiCommand>,
        reg_state: Arc<Mutex<RegistrationState>>,
        active_calls: Arc<Mutex<Vec<Call>>>,
    ) -> Self {
        let initial_account = initial_config.accounts.first().cloned().unwrap_or_else(|| {
            Account {
                name: "Default".to_string(),
                username: "user".to_string(),
                domain: "example.com".to_string(),
                password: Some("pass".to_string()),
                proxy: None,
            }
        });

        Self {
            account_name: initial_account.name,
            account_username: initial_account.username,
            account_domain: initial_account.domain,
            account_password: initial_account.password.unwrap_or_default(),
            bind_address: initial_config.connection.bind_address,
            transport_type: initial_config.connection.transport_type,
            dialer_input: String::new(),
            call_history: Vec::new(),
            reg_state,
            active_calls,
            command_sender,
        }
    }

    fn render_status_badge(&self, ui: &mut egui::Ui) {
        let state = self.reg_state.lock().unwrap().clone();
        let (text, color) = match state {
            RegistrationState::Unregistered => ("Offline", egui::Color32::from_rgb(150, 150, 150)),
            RegistrationState::Registering => ("Connecting...", egui::Color32::from_rgb(255, 215, 0)),
            RegistrationState::Registered => ("Online", egui::Color32::from_rgb(50, 205, 50)),
            RegistrationState::Failed(_) => ("Error", egui::Color32::from_rgb(220, 20, 60)),
        };

        egui::Frame::none()
            .fill(color.gamma_multiply(0.2))
            .stroke(egui::Stroke::new(1.0, color))
            .rounding(10.0)
            .inner_margin(egui::Margin::symmetric(10.0, 4.0))
            .show(ui, |ui| {
                ui.label(egui::RichText::new(text).color(color).strong());
            });
    }

    fn render_dialer(&mut self, ui: &mut egui::Ui) {
        ui.vertical_centered(|ui| {
            egui::Frame::none()
                .fill(ui.visuals().widgets.noninteractive.bg_fill)
                .rounding(8.0)
                .inner_margin(20.0)
                .show(ui, |ui| {
                    ui.set_max_width(400.0);
                    ui.label("Enter Number or SIP URI");
                    ui.add_space(10.0);

                    ui.horizontal(|ui| {
                        let text_edit = egui::TextEdit::singleline(&mut self.dialer_input)
                            .hint_text("sip:user@domain.com")
                            .font(egui::FontId::proportional(18.0));

                        ui.add_sized([ui.available_width() - 80.0, 40.0], text_edit);

                        if ui.add_sized([70.0, 40.0], egui::Button::new(egui::RichText::new("CALL").strong()))
                            .clicked()
                        {
                            if !self.dialer_input.is_empty() {
                                let destination = self.dialer_input.clone();
                                self.call_history.push(destination.clone());
                                let _ = self.command_sender.send(UiCommand::Invite(destination));
                            }
                        }
                    });
                });
        });
    }

    fn render_active_calls(&mut self, ui: &mut egui::Ui) {
        let calls = self.active_calls.lock().unwrap().clone();

        ui.group(|ui| {
            ui.set_min_width(ui.available_width());
            ui.label(egui::RichText::new("Active Calls").strong());
            ui.add_space(5.0);

            if calls.is_empty() {
                ui.label(egui::RichText::new("No active calls").weak());
            } else {
                for call in calls {
                    egui::Frame::none()
                        .fill(ui.visuals().widgets.inactive.bg_fill)
                        .rounding(4.0)
                        .inner_margin(10.0)
                        .show(ui, |ui| {
                            ui.horizontal(|ui| {
                                ui.vertical(|ui| {
                                    ui.label(egui::RichText::new(&call.remote_uri).strong());
                                    ui.label(egui::RichText::new(format!("{:?}", call.state)).small());
                                });
                                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                                    if ui.button(egui::RichText::new("Hangup").color(egui::Color32::RED)).clicked() {
                                        let _ = self.command_sender.send(UiCommand::Hangup(call.id));
                                    }
                                });
                            });
                        });
                    ui.add_space(5.0);
                }
            }
        });
    }

    fn render_call_history(&mut self, ui: &mut egui::Ui) {
        ui.collapsing(egui::RichText::new("Recent Calls").strong(), |ui| {
            if self.call_history.is_empty() {
                ui.label(egui::RichText::new("Empty").weak());
            } else {
                for entry in self.call_history.iter().rev() {
                    ui.label(entry);
                }
            }
        });
    }
}
