use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

#[derive(Debug, Default)]
pub struct Composer {
    pub text: String,
}

impl Composer {
    pub fn handle_key(&mut self, key: KeyEvent) -> Option<String> {
        match key.code {
            KeyCode::Enter => (!self.text.is_empty()).then(|| std::mem::take(&mut self.text)),
            KeyCode::Backspace => {
                self.text.pop();
                None
            }
            KeyCode::Char(ch)
                if !key
                    .modifiers
                    .intersects(KeyModifiers::CONTROL | KeyModifiers::ALT) =>
            {
                self.text.push(ch);
                None
            }
            _ => None,
        }
    }
}
