//! 常驻终端宠物与宠物选择器的聊天 widget 辅助函数。

use super::*;
use crate::config_compat::types::TuiPetAnchor;

pub(super) fn load_ambient_pet(
    config: &Config,
    frame_requester: FrameRequester,
) -> Option<crate::tui_core::pets::AmbientPet> {
    let selected_pet = config.tui_pet.as_deref()?;
    if selected_pet == crate::tui_core::pets::DISABLED_PET_ID {
        return None;
    }

    crate::tui_core::pets::AmbientPet::load(
        Some(selected_pet),
        &config.reflect_home,
        frame_requester,
        config.animations,
    )
    .ok()
}

pub(super) fn start_configured_pet_load_if_needed(
    config: &Config,
    ambient_pet_missing: bool,
    frame_requester: FrameRequester,
    app_event_tx: AppEventSender,
) {
    let Some(pet_id) = config.tui_pet.clone() else {
        return;
    };
    if pet_id == crate::tui_core::pets::DISABLED_PET_ID || !ambient_pet_missing {
        return;
    }

    let reflect_home = config.reflect_home.clone();
    let animations_enabled = config.animations;
    spawn_pet_load(move || {
        let result = crate::tui_core::pets::ensure_builtin_pack_for_pet(&pet_id, &reflect_home)
            .and_then(|()| {
                crate::tui_core::pets::AmbientPet::load(
                    Some(&pet_id),
                    &reflect_home,
                    frame_requester,
                    animations_enabled,
                )
            })
            .map(Some)
            .map_err(|err| err.to_string());
        app_event_tx.send(AppEvent::ConfiguredPetLoaded { pet_id, result });
    });
}

impl ChatWidget {
    pub(super) fn set_ambient_pet_notification(
        &mut self,
        kind: crate::tui_core::pets::PetNotificationKind,
        body: Option<String>,
    ) {
        if let Some(pet) = self.ambient_pet.as_mut() {
            pet.set_notification(kind, body);
        }
    }

    pub(crate) fn ambient_pet_image_enabled(&self) -> bool {
        self.ambient_pet
            .as_ref()
            .is_some_and(crate::tui_core::pets::AmbientPet::image_enabled)
    }

    pub(crate) fn disable_ambient_pet_for_session(&mut self) {
        self.ambient_pet = None;
        self.request_redraw();
    }

    pub(crate) fn ambient_pet_draw(
        &self,
        area: Rect,
        composer_bottom_y: u16,
    ) -> Option<crate::tui_core::pets::AmbientPetDraw> {
        if !self.bottom_pane.no_modal_or_popup_active() {
            return None;
        }

        let anchor_bottom_y = match self.config.tui_pet_anchor {
            TuiPetAnchor::Composer => composer_bottom_y,
            TuiPetAnchor::ScreenBottom => area.bottom(),
        };
        self.ambient_pet
            .as_ref()?
            .draw_request(area, anchor_bottom_y)
    }

    pub(super) fn ambient_pet_wrap_reserved_cols(&self) -> u16 {
        self.ambient_pet
            .as_ref()
            .filter(|pet| pet.image_enabled())
            .map(|pet| {
                pet.image_columns()
                    .saturating_add(AMBIENT_PET_WRAP_GAP_COLUMNS)
            })
            .unwrap_or(0)
    }

    pub(crate) fn history_wrap_width(&self, width: u16) -> u16 {
        width
            .saturating_sub(self.ambient_pet_wrap_reserved_cols())
            .max(1)
    }

    pub(crate) fn pet_picker_preview_draw(&self) -> Option<crate::tui_core::pets::AmbientPetDraw> {
        self.bottom_pane
            .selected_index_for_active_view(crate::tui_core::pets::PET_PICKER_VIEW_ID)?;
        let area = self.pet_picker_preview_state.area()?;
        let request = self
            .pet_picker_preview_pet
            .as_ref()?
            .preview_draw_request(area)?;
        self.pet_picker_preview_image_visible.set(true);
        Some(request)
    }

    pub(crate) fn should_clear_pet_picker_preview_image(&self) -> bool {
        self.pet_picker_preview_image_visible.replace(false)
    }

    pub(crate) fn fail_pet_picker_preview_render(&mut self, message: String) {
        self.pet_picker_preview_state.set_error(message);
        self.pet_picker_preview_pet = None;
        self.request_redraw();
    }

    pub(crate) fn open_pets_picker(&mut self) {
        if self.warn_if_pets_unsupported() {
            return;
        }

        self.pet_picker_preview_state.clear();
        self.pet_picker_preview_pet = None;
        let params = crate::tui_core::pets::build_pet_picker_params(
            self.config.tui_pet.as_deref(),
            &self.config.reflect_home,
            self.pet_picker_preview_state.clone(),
        );
        self.bottom_pane.show_selection_view(params);
        let initial_pet_id = self
            .config
            .tui_pet
            .as_deref()
            .unwrap_or(crate::tui_core::pets::DEFAULT_PET_ID)
            .to_string();
        self.start_pet_picker_preview(initial_pet_id);
    }

    pub(crate) fn select_pet_by_id(&mut self, pet_id: String) {
        if self.warn_if_pets_unsupported() {
            return;
        }

        self.app_event_tx.send(AppEvent::PetSelected { pet_id });
    }

    fn warn_if_pets_unsupported(&mut self) -> bool {
        let support = self.pet_image_support();
        let Some(message) = support.unsupported_message() else {
            return false;
        };

        self.add_warning_message(message.to_string());
        true
    }

    fn pet_image_support(&self) -> crate::tui_core::pets::PetImageSupport {
        #[cfg(test)]
        if let Some(support) = self.pet_image_support_override {
            return support;
        }

        #[cfg(test)]
        return crate::tui_core::pets::PetImageSupport::Unsupported(
            crate::tui_core::pets::PetImageUnsupportedReason::Terminal,
        );

        #[cfg(not(test))]
        crate::tui_core::pets::detect_pet_image_support()
    }

    /// 设置该 widget 配置副本中由 TUI 选择器预选的宠物。
    pub(crate) fn set_tui_pet(&mut self, pet: Option<String>) {
        self.config.tui_pet = pet;
        self.ambient_pet = load_ambient_pet(&self.config, self.frame_requester.clone());
        self.apply_ambient_pet_image_support_override_for_tests();
        self.request_redraw();
    }

    pub(crate) fn set_tui_pet_loaded(
        &mut self,
        pet: Option<String>,
        ambient_pet: Option<crate::tui_core::pets::AmbientPet>,
    ) {
        self.config.tui_pet = pet;
        self.ambient_pet = ambient_pet;
        self.apply_ambient_pet_image_support_override_for_tests();
        self.request_redraw();
    }

    #[cfg(test)]
    fn apply_ambient_pet_image_support_override_for_tests(&mut self) {
        if let Some(support) = self.pet_image_support_override
            && let Some(pet) = self.ambient_pet.as_mut()
        {
            pet.set_image_support_for_tests(support);
        }
    }

    #[cfg(not(test))]
    fn apply_ambient_pet_image_support_override_for_tests(&mut self) {}

    pub(crate) fn start_pet_picker_preview(&mut self, pet_id: String) {
        self.pet_picker_preview_request_id =
            self.pet_picker_preview_request_id.wrapping_add(/*rhs*/ 1);
        let request_id = self.pet_picker_preview_request_id;
        self.pet_picker_preview_pet = None;
        if pet_id == crate::tui_core::pets::DISABLED_PET_ID {
            self.pet_picker_preview_state.set_disabled();
            self.request_redraw();
            return;
        }

        self.pet_picker_preview_state.set_loading();
        self.request_redraw();

        let reflect_home = self.config.reflect_home.clone();
        let frame_requester = self.frame_requester.clone();
        let tx = self.app_event_tx.clone();
        spawn_pet_load(move || {
            let result = crate::tui_core::pets::ensure_builtin_pack_for_pet(&pet_id, &reflect_home)
                .and_then(|()| {
                    crate::tui_core::pets::AmbientPet::load(
                        Some(&pet_id),
                        &reflect_home,
                        frame_requester,
                        /*animations_enabled*/ false,
                    )
                })
                .map_err(|err| err.to_string());
            tx.send(AppEvent::PetPreviewLoaded { request_id, result });
        });
    }

    pub(crate) fn finish_pet_picker_preview_load(
        &mut self,
        request_id: u64,
        result: Result<crate::tui_core::pets::AmbientPet, String>,
    ) {
        if request_id != self.pet_picker_preview_request_id {
            return;
        }

        match result {
            Ok(pet) => {
                self.pet_picker_preview_state.set_ready();
                self.pet_picker_preview_pet = Some(pet);
                #[cfg(test)]
                if let Some(support) = self.pet_image_support_override
                    && let Some(pet) = self.pet_picker_preview_pet.as_mut()
                {
                    pet.set_image_support_for_tests(support);
                }
            }
            Err(message) => {
                self.pet_picker_preview_state.set_error(message);
                self.pet_picker_preview_pet = None;
            }
        }
        self.request_redraw();
    }

    pub(crate) fn show_pet_selection_loading_popup(&mut self) -> u64 {
        self.pet_selection_load_request_id =
            self.pet_selection_load_request_id.wrapping_add(/*rhs*/ 1);
        self.pet_picker_preview_state.clear();
        self.pet_picker_preview_pet = None;
        self.bottom_pane.show_selection_view(SelectionViewParams {
            view_id: Some(PET_SELECTION_LOADING_VIEW_ID),
            title: Some("Loading Pet".to_string()),
            subtitle: Some("Preparing the terminal pet.".to_string()),
            items: vec![SelectionItem {
                name: "Loading selected pet...".to_string(),
                is_disabled: true,
                ..Default::default()
            }],
            ..Default::default()
        });
        self.pet_selection_load_request_id
    }

    pub(crate) fn finish_pet_selection_loading_popup(&mut self, request_id: u64) -> bool {
        if request_id != self.pet_selection_load_request_id {
            return false;
        }
        self.bottom_pane
            .dismiss_active_view_if_id(PET_SELECTION_LOADING_VIEW_ID);
        true
    }

    #[cfg(test)]
    pub(crate) fn set_pet_image_support_for_tests(
        &mut self,
        support: crate::tui_core::pets::PetImageSupport,
    ) {
        self.pet_image_support_override = Some(support);
        self.apply_ambient_pet_image_support_override_for_tests();
    }

    #[cfg(test)]
    pub(crate) fn install_test_ambient_pet_for_tests(&mut self, animations_enabled: bool) {
        self.set_tui_pet_loaded(
            Some("test".to_string()),
            Some(crate::tui_core::pets::test_ambient_pet(
                self.frame_requester.clone(),
                animations_enabled,
            )),
        );
    }
}

fn spawn_pet_load(f: impl FnOnce() + Send + 'static) {
    if let Ok(handle) = tokio::runtime::Handle::try_current() {
        std::mem::drop(handle.spawn_blocking(f));
    } else {
        let _ = std::thread::spawn(f);
    }
}
