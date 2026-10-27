//! 审批相关 app-server 载荷的窄转换辅助函数。
//!
//! TUI 大多保持 app-server 审批类型不变。这些辅助函数覆盖
//! 其余场景：UI 消费私有文件变更显示
//! 模型，或需要把已授予的权限响应转换为出站
//! 提交。

use crate::app_server_protocol::AdditionalNetworkPermissions;
use crate::app_server_protocol::FileUpdateChange;
use crate::app_server_protocol::GrantedPermissionProfile;
use crate::app_server_protocol::PatchChangeKind;
use crate::protocol_compat::request_permissions::RequestPermissionProfile as CoreRequestPermissionProfile;
use crate::tui_core::diff_model::FileChange;
use std::collections::HashMap;
use std::path::PathBuf;

pub(crate) fn granted_permission_profile_from_request(
    value: CoreRequestPermissionProfile,
) -> GrantedPermissionProfile {
    GrantedPermissionProfile {
        network: value.network.map(|network| AdditionalNetworkPermissions {
            enabled: network.enabled,
        }),
        file_system: value.file_system.map(Into::into),
    }
}

pub(crate) fn file_update_changes_to_display(
    changes: Vec<FileUpdateChange>,
) -> HashMap<PathBuf, FileChange> {
    changes
        .into_iter()
        .map(|change| {
            let path = PathBuf::from(change.path);
            let file_change = match change.kind {
                PatchChangeKind::Add => FileChange::Add {
                    content: change.diff,
                },
                PatchChangeKind::Delete => FileChange::Delete {
                    content: change.diff,
                },
                PatchChangeKind::Update { move_path } => FileChange::Update {
                    unified_diff: change.diff,
                    move_path,
                },
            };
            (path, file_change)
        })
        .collect()
}

#[cfg(all(test, feature = "tui-upstream-tests"))]
mod tests {
    use super::file_update_changes_to_display;
    use super::granted_permission_profile_from_request;
    use crate::app_server_protocol::AdditionalFileSystemPermissions;
    use crate::app_server_protocol::AdditionalNetworkPermissions;
    use crate::app_server_protocol::FileSystemAccessMode;
    use crate::app_server_protocol::FileSystemPath;
    use crate::app_server_protocol::FileSystemSandboxEntry;
    use crate::app_server_protocol::FileSystemSpecialPath;
    use crate::app_server_protocol::FileUpdateChange;
    use crate::app_server_protocol::GrantedPermissionProfile;
    use crate::app_server_protocol::PatchChangeKind;
    use crate::app_server_protocol::RequestPermissionProfile;
    use crate::protocol_compat::request_permissions::RequestPermissionProfile as CoreRequestPermissionProfile;
    use crate::tui_core::diff_model::FileChange;
    use crate::utils_absolute_path::AbsolutePathBuf;
    use pretty_assertions::assert_eq;
    use std::collections::HashMap;
    use std::path::PathBuf;

    fn absolute_path(path: &str) -> AbsolutePathBuf {
        AbsolutePathBuf::try_from(PathBuf::from(path)).expect("path must be absolute")
    }

    #[test]
    fn converts_file_update_changes_to_display() {
        assert_eq!(
            file_update_changes_to_display(vec![FileUpdateChange {
                path: "foo.txt".to_string(),
                kind: PatchChangeKind::Add,
                diff: "hello\n".to_string(),
            }]),
            HashMap::from([(
                PathBuf::from("foo.txt"),
                FileChange::Add {
                    content: "hello\n".to_string(),
                },
            )])
        );
    }

    #[test]
    fn converts_request_permissions_into_granted_permissions() {
        let request = RequestPermissionProfile {
            network: Some(AdditionalNetworkPermissions {
                enabled: Some(true),
            }),
            file_system: Some(AdditionalFileSystemPermissions {
                read: Some(vec![absolute_path("/tmp/read-only").into()]),
                write: Some(vec![absolute_path("/tmp/write").into()]),
                glob_scan_max_depth: None,
                entries: None,
            }),
        };
        let request = CoreRequestPermissionProfile::try_from(request)
            .expect("API paths should convert to native paths");

        assert_eq!(
            granted_permission_profile_from_request(request),
            GrantedPermissionProfile {
                network: Some(AdditionalNetworkPermissions {
                    enabled: Some(true),
                }),
                file_system: Some(AdditionalFileSystemPermissions {
                    read: Some(vec![absolute_path("/tmp/read-only").into()]),
                    write: Some(vec![absolute_path("/tmp/write").into()]),
                    glob_scan_max_depth: None,
                    entries: Some(vec![
                        FileSystemSandboxEntry {
                            path: FileSystemPath::Path {
                                path: absolute_path("/tmp/read-only").into(),
                            },
                            access: FileSystemAccessMode::Read,
                        },
                        FileSystemSandboxEntry {
                            path: FileSystemPath::Path {
                                path: absolute_path("/tmp/write").into(),
                            },
                            access: FileSystemAccessMode::Write,
                        },
                    ]),
                }),
            }
        );
    }

    #[test]
    fn converts_request_permissions_into_canonical_granted_permissions() {
        let request = RequestPermissionProfile {
            network: None,
            file_system: Some(AdditionalFileSystemPermissions {
                read: None,
                write: None,
                glob_scan_max_depth: None,
                entries: Some(vec![FileSystemSandboxEntry {
                    path: FileSystemPath::Special {
                        value: FileSystemSpecialPath::Root,
                    },
                    access: FileSystemAccessMode::Write,
                }]),
            }),
        };
        let request = CoreRequestPermissionProfile::try_from(request)
            .expect("API paths should convert to native paths");

        assert_eq!(
            granted_permission_profile_from_request(request),
            GrantedPermissionProfile {
                network: None,
                file_system: Some(AdditionalFileSystemPermissions {
                    read: None,
                    write: None,
                    glob_scan_max_depth: None,
                    entries: Some(vec![FileSystemSandboxEntry {
                        path: FileSystemPath::Special {
                            value: FileSystemSpecialPath::Root,
                        },
                        access: FileSystemAccessMode::Write,
                    }]),
                }),
            }
        );
    }
}
