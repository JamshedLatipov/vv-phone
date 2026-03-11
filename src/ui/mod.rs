pub struct SoftphoneApp {
    pub dialer_input: String,
    pub call_history: Vec<String>,
}

impl Default for SoftphoneApp {
    fn default() -> Self {
        Self {
            dialer_input: String::new(),
            call_history: Vec::new(),
        }
    }
}

// In a real implementation with egui, we would implement eframe::App here.
// But we cannot compile egui in this environment.
