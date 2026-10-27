pub(crate) struct SessionHeader {
    model: String,
}

impl SessionHeader {
    pub(crate) fn new(model: String) -> Self {
        Self { model }
    }

    /// 更新头部显示的模型文本。
    pub(crate) fn set_model(&mut self, model: &str) {
        if self.model != model {
            self.model = model.to_string();
        }
    }
}
