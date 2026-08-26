//! 内置宠物资产获取与缓存所有权。
//!
//! 与自定义宠物不同，内置宠物不会作为本地 sprite 表
//! 检入 TUI 包。Reflect 不再从远程 CDN 获取内置宠物 sprite 表，
//! 因此只有当之前安装的 sprite 表已存在并通过结构校验时，
//! 解析才会成功——它位于 REFLECT_HOME 下的版本化缓存中。
//! 当没有缓存资产时，解析会优雅失败，使宠物降级为不可用状态，
//! 而不是去连外部端点。
//!
//! 本模块刻意止步于“此路径存在经校验的 sprite 表”。
//! 更上层负责决定预览何时应阻塞在解析上，以及成功加载的
//! 内置宠物何时才安全地持久化到配置。

use std::path::Path;
use std::path::PathBuf;

use anyhow::Context;
use anyhow::Result;
use anyhow::bail;

use super::catalog;

const PET_PACK_VERSION: &str = "v1";
const PET_PACK_DIR: &str = "cache/tui-pets";

pub(crate) fn builtin_spritesheet_path(reflect_home: &Path, file: &str) -> PathBuf {
    pack_dir(reflect_home).join("assets").join(file)
}

/// 确保内置宠物的 sprite 表存在且结构有效。
///
/// Reflect 不从远程 CDN 下载内置宠物资产。仅当缓存中已存在
/// 先前安装的 sprite 表并通过预期几何校验时，解析
/// 才会成功。否则返回错误，使调用方可以优雅降级。
pub(crate) fn ensure_builtin_pet(reflect_home: &Path, pet: catalog::BuiltinPet) -> Result<()> {
    let destination = builtin_spritesheet_path(reflect_home, pet.spritesheet_file);
    if validate_cached_spritesheet(&destination).is_ok() {
        return Ok(());
    }
    bail!(
        "built-in pet spritesheet '{}' is not available locally and Reflect does not fetch pet assets from a remote CDN",
        pet.spritesheet_file
    );
}

#[cfg(test)]
fn builtin_pet_url(pet: catalog::BuiltinPet) -> Result<String> {
    // Reflect 不发布宠物资产 CDN。因测试需要而保留结构兼容；
    // 解析始终失败。
    let _ = pet;
    bail!("Reflect does not expose a built-in pet asset CDN URL");
}

fn pack_dir(reflect_home: &Path) -> PathBuf {
    reflect_home.join(PET_PACK_DIR).join(PET_PACK_VERSION)
}

fn validate_cached_spritesheet(path: &Path) -> Result<()> {
    let (width, height) =
        image::image_dimensions(path).with_context(|| format!("read {}", path.display()))?;
    if width != catalog::SPRITESHEET_WIDTH || height != catalog::SPRITESHEET_HEIGHT {
        bail!(
            "invalid pet spritesheet dimensions for {}: expected {}x{}, got {}x{}",
            path.display(),
            catalog::SPRITESHEET_WIDTH,
            catalog::SPRITESHEET_HEIGHT,
            width,
            height
        );
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn write_test_pack(reflect_home: &Path) {
    use std::fs;
    let assets_dir = pack_dir(reflect_home).join("assets");
    fs::create_dir_all(&assets_dir).unwrap();
    for pet in catalog::BUILTIN_PETS {
        let path = assets_dir.join(pet.spritesheet_file);
        catalog::write_test_spritesheet(&path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin_pet_url_has_no_remote_cdn() {
        // Reflect 不暴露内置宠物资产 CDN，因此解析必须
        // 失败而不是产生远程 URL。
        let pet = catalog::builtin_pet("dewey").unwrap();

        assert!(builtin_pet_url(pet).is_err());
    }

    #[test]
    fn write_test_pack_installs_all_builtins() {
        let dir = tempfile::tempdir().unwrap();

        write_test_pack(dir.path());

        for pet in catalog::BUILTIN_PETS {
            let path = builtin_spritesheet_path(dir.path(), pet.spritesheet_file);
            assert!(path.is_file());
            validate_cached_spritesheet(&path).unwrap();
        }
    }
}
