//! 底栏快捷键目录：ShortcutId / ShortcutBinding / DisplayCondition
//! / ShortcutDescriptor 类型与 SHORTCUTS 常量表。从 footer.rs 抽出。
//!
//! 依赖父模块的 import（key_hint / KeyCode / KeyBinding 等，经 `use super::*;`）。

use super::*;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ShortcutId {
    Commands,
    ShellCommands,
    InsertNewline,
    QueueMessageTab,
    FilePaths,
    PasteImage,
    ExternalEditor,
    EditPrevious,
    HistorySearch,
    Quit,
    ShowTranscript,
    ChangeMode,
    ReasoningDown,
    ReasoningUp,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ShortcutBinding {
    pub(super) key: KeyBinding,
    pub(super) condition: DisplayCondition,
}

impl ShortcutBinding {
    pub(super) fn matches(&self, state: ShortcutsState) -> bool {
        self.condition.matches(state)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum DisplayCondition {
    Always,
    WhenShiftEnterHint,
    WhenNotShiftEnterHint,
    WhenUnderWSL,
    WhenCollaborationModesEnabled,
}

impl DisplayCondition {
    pub(super) fn matches(self, state: ShortcutsState) -> bool {
        match self {
            DisplayCondition::Always => true,
            DisplayCondition::WhenShiftEnterHint => state.use_shift_enter_hint,
            DisplayCondition::WhenNotShiftEnterHint => !state.use_shift_enter_hint,
            DisplayCondition::WhenUnderWSL => state.is_wsl,
            DisplayCondition::WhenCollaborationModesEnabled => state.collaboration_modes_enabled,
        }
    }
}

pub(super) struct ShortcutDescriptor {
    pub(super) id: ShortcutId,
    pub(super) bindings: &'static [ShortcutBinding],
    prefix: &'static str,
    label: &'static str,
}

impl ShortcutDescriptor {
    pub(super) fn binding_for(&self, state: ShortcutsState) -> Option<&'static ShortcutBinding> {
        self.bindings.iter().find(|binding| binding.matches(state))
    }

    pub(super) fn overlay_entry(&self, state: ShortcutsState) -> Option<Line<'static>> {
        let key = match self.id {
            ShortcutId::InsertNewline => state.key_hints.insert_newline,
            ShortcutId::QueueMessageTab => state.key_hints.queue,
            ShortcutId::ExternalEditor => state.key_hints.external_editor,
            ShortcutId::EditPrevious => state.key_hints.edit_previous,
            ShortcutId::ShowTranscript => state.key_hints.show_transcript,
            ShortcutId::HistorySearch => state.key_hints.history_search,
            ShortcutId::ReasoningDown => state.key_hints.reasoning_down,
            ShortcutId::ReasoningUp => state.key_hints.reasoning_up,
            ShortcutId::Commands
            | ShortcutId::ShellCommands
            | ShortcutId::FilePaths
            | ShortcutId::PasteImage
            | ShortcutId::Quit
            | ShortcutId::ChangeMode => self.binding_for(state).map(|binding| binding.key),
        }?;
        let mut line = Line::from(vec![self.prefix.into(), key.into()]);
        match self.id {
            ShortcutId::QueueMessageTab => {
                if state.is_task_running || state.queue_submissions {
                    line.push_span(" to queue message");
                } else {
                    line.push_span(" to submit message");
                }
            }
            ShortcutId::EditPrevious => {
                if state.esc_backtrack_hint {
                    line.push_span(" again to edit previous message");
                } else {
                    line.extend(vec![
                        " ".into(),
                        key.into(),
                        " to edit previous message".into(),
                    ]);
                }
            }
            ShortcutId::Quit => {
                if state.is_task_running {
                    line.push_span(" to interrupt");
                } else {
                    line.push_span(" to exit");
                }
            }
            _ => line.push_span(self.label),
        };
        Some(line)
    }
}

pub(super) const SHORTCUTS: &[ShortcutDescriptor] = &[
    ShortcutDescriptor {
        id: ShortcutId::Commands,
        bindings: &[ShortcutBinding {
            key: key_hint::plain(KeyCode::Char('/')),
            condition: DisplayCondition::Always,
        }],
        prefix: "",
        label: " for commands",
    },
    ShortcutDescriptor {
        id: ShortcutId::ShellCommands,
        bindings: &[ShortcutBinding {
            key: key_hint::plain(KeyCode::Char('!')),
            condition: DisplayCondition::Always,
        }],
        prefix: "",
        label: " for shell commands",
    },
    ShortcutDescriptor {
        id: ShortcutId::InsertNewline,
        bindings: &[
            ShortcutBinding {
                key: key_hint::shift(KeyCode::Enter),
                condition: DisplayCondition::WhenShiftEnterHint,
            },
            ShortcutBinding {
                key: key_hint::ctrl(KeyCode::Char('j')),
                condition: DisplayCondition::WhenNotShiftEnterHint,
            },
        ],
        prefix: "",
        label: " for newline",
    },
    ShortcutDescriptor {
        id: ShortcutId::QueueMessageTab,
        bindings: &[ShortcutBinding {
            key: key_hint::plain(KeyCode::Tab),
            condition: DisplayCondition::Always,
        }],
        prefix: "",
        label: " to queue message",
    },
    ShortcutDescriptor {
        id: ShortcutId::FilePaths,
        bindings: &[ShortcutBinding {
            key: key_hint::plain(KeyCode::Char('@')),
            condition: DisplayCondition::Always,
        }],
        prefix: "",
        label: " for file paths",
    },
    ShortcutDescriptor {
        id: ShortcutId::PasteImage,
        // 在 WSL 下运行时显示 Ctrl+Alt+V（终端通常会拦截单纯的 Ctrl+V）；
        // 否则回退到 Ctrl+V。
        bindings: &[
            ShortcutBinding {
                key: key_hint::ctrl_alt(KeyCode::Char('v')),
                condition: DisplayCondition::WhenUnderWSL,
            },
            ShortcutBinding {
                key: key_hint::ctrl(KeyCode::Char('v')),
                condition: DisplayCondition::Always,
            },
        ],
        prefix: "",
        label: " to paste images",
    },
    ShortcutDescriptor {
        id: ShortcutId::ExternalEditor,
        bindings: &[ShortcutBinding {
            key: key_hint::ctrl(KeyCode::Char('g')),
            condition: DisplayCondition::Always,
        }],
        prefix: "",
        label: " to edit in external editor",
    },
    ShortcutDescriptor {
        id: ShortcutId::EditPrevious,
        bindings: &[ShortcutBinding {
            key: key_hint::plain(KeyCode::Esc),
            condition: DisplayCondition::Always,
        }],
        prefix: "",
        label: "",
    },
    ShortcutDescriptor {
        id: ShortcutId::HistorySearch,
        bindings: &[ShortcutBinding {
            key: key_hint::ctrl(KeyCode::Char('r')),
            condition: DisplayCondition::Always,
        }],
        prefix: "",
        label: " search history",
    },
    ShortcutDescriptor {
        id: ShortcutId::Quit,
        bindings: &[ShortcutBinding {
            key: key_hint::ctrl(KeyCode::Char('c')),
            condition: DisplayCondition::Always,
        }],
        prefix: "",
        label: " to exit",
    },
    ShortcutDescriptor {
        id: ShortcutId::ShowTranscript,
        bindings: &[ShortcutBinding {
            key: key_hint::ctrl(KeyCode::Char('t')),
            condition: DisplayCondition::Always,
        }],
        prefix: "",
        label: " to view transcript",
    },
    ShortcutDescriptor {
        id: ShortcutId::ChangeMode,
        bindings: &[ShortcutBinding {
            key: key_hint::shift(KeyCode::Tab),
            condition: DisplayCondition::WhenCollaborationModesEnabled,
        }],
        prefix: "",
        label: " to change mode",
    },
    ShortcutDescriptor {
        id: ShortcutId::ReasoningDown,
        bindings: &[ShortcutBinding {
            key: key_hint::alt(KeyCode::Char(',')),
            condition: DisplayCondition::Always,
        }],
        prefix: "",
        label: " reasoning down",
    },
    ShortcutDescriptor {
        id: ShortcutId::ReasoningUp,
        bindings: &[ShortcutBinding {
            key: key_hint::alt(KeyCode::Char('.')),
            condition: DisplayCondition::Always,
        }],
        prefix: "",
        label: " reasoning up",
    },
];
