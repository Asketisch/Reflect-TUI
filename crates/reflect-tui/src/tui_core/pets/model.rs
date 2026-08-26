//! Pet 清单加载与归一化。
//!
//! 本模块将多种用户可见的选择器转换为一种归一化的内存 `Pet`：内置目录 id、
//! REFLECT_HOME 下的自定义 pet id、遗留头像目录，以及供测试或
//! 本地迭代使用的显式文件系统路径。
//!
//! 关键不变量是：每个返回的 `Pet` 都指向一个已经存在且尺寸兼容 
//! 应用程序的本地 sprite 表路径。资产获取刻意不在本模块范围内；
//! 调用方必须确保内置 pet 在请求模型层加载前已被下载。

use std::collections::HashMap;
use std::fs;
use std::path::Component;
use std::path::Path;
use std::path::PathBuf;
use std::time::Duration;

use anyhow::Context;
use anyhow::Result;
use anyhow::bail;
use serde::Deserialize;
use sha2::Digest as _;
use sha2::Sha256;

use super::catalog;

const MAX_PET_FRAMES: usize = 256;
const MAX_ANIMATION_FPS: f64 = 60.0;

#[derive(Debug, Clone)]
pub struct AnimationFrame {
    pub sprite_index: usize,
    pub duration: Duration,
}

#[derive(Debug, Clone)]
pub struct Animation {
    pub frames: Vec<AnimationFrame>,
    pub loop_start: Option<usize>,
    pub fallback: String,
}

impl Animation {
    pub(super) fn total_duration(&self) -> Duration {
        self.frames
            .iter()
            .map(|frame| frame.duration)
            .sum::<Duration>()
    }
}

/// pet sprite 表的一条具名动画轨道。
///
/// 轨道使用已解码帧网格中的 sprite 索引，并为一次性序列提供
/// 回退动画名。调用方不应仅因为动画有多个帧就假设它会循环；
/// `loop_start == None` 表示最后一帧最终会交接到 `fallback`。
#[derive(Debug, Clone)]
pub struct Pet {
    pub id: String,
    pub display_name: String,
    pub description: Option<String>,
    pub spritesheet_path: PathBuf,
    pub frame_width: u32,
    pub frame_height: u32,
    pub columns: u32,
    pub rows: u32,
    pub frame_count: usize,
    pub animations: HashMap<String, Animation>,
}

impl Pet {
    /// 将 pet 选择器加载为具体的本地 pet 定义。
    ///
    /// 选择器可以是内置目录 pet、自定义 pet id、遗留
    /// 头像 id 或显式路径。本方法假定任何内置资产
    /// 已物化到 REFLECT_HOME；若调用方跳过了
    /// 资产获取步骤，则首次使用时会在缺失 sprite 表处报错。
    pub(super) fn load_with_reflect_home(value: &str, reflect_home: Option<&Path>) -> Result<Self> {
        if path_like(value) {
            return load_pet_path(value);
        }

        if let Some(custom_id) = value.strip_prefix(CUSTOM_PET_PREFIX) {
            return load_custom_pet(custom_id, reflect_home);
        }

        if let Some(builtin) = catalog::builtin_pet(value) {
            return load_builtin_pet(builtin, reflect_home);
        }

        load_custom_pet(value, reflect_home)
    }

    pub fn frame_count(&self) -> usize {
        self.frame_count
    }

    pub(super) fn frame_cache_key(&self) -> Result<String> {
        let bytes = fs::read(&self.spritesheet_path)
            .with_context(|| format!("read {}", self.spritesheet_path.display()))?;
        let digest = Sha256::digest(&bytes);
        Ok(format!(
            "sha256-{digest:x}-{}x{}-{}x{}",
            self.frame_width, self.frame_height, self.columns, self.rows
        ))
    }
}

pub(super) const CUSTOM_PET_PREFIX: &str = "custom:";

#[derive(Debug, Deserialize)]
struct PetFile {
    #[serde(default)]
    id: Option<String>,
    #[serde(default, rename = "displayName")]
    display_name: Option<String>,
    #[serde(default)]
    description: Option<String>,
    #[serde(default, rename = "spritesheetPath")]
    spritesheet_path: Option<String>,
    frame: Option<FrameSpec>,
    #[serde(default)]
    animations: HashMap<String, AnimationSpec>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
struct FrameSpec {
    width: u32,
    height: u32,
    columns: u32,
    rows: u32,
}

impl Default for FrameSpec {
    fn default() -> Self {
        Self {
            width: catalog::DEFAULT_FRAME_WIDTH,
            height: catalog::DEFAULT_FRAME_HEIGHT,
            columns: catalog::DEFAULT_FRAME_COLUMNS,
            rows: catalog::DEFAULT_FRAME_ROWS,
        }
    }
}

pub(super) fn custom_pet_selector(id: &str) -> String {
    format!("{CUSTOM_PET_PREFIX}{id}")
}

#[derive(Debug, Deserialize)]
struct AnimationSpec {
    #[serde(default)]
    frames: Vec<usize>,
    fps: Option<f64>,
    #[serde(rename = "loop")]
    loop_animation: Option<bool>,
    #[serde(default)]
    fallback: String,
}

fn load_builtin_pet(pet: catalog::BuiltinPet, reflect_home: Option<&Path>) -> Result<Pet> {
    let reflect_home = reflect_home.context("REFLECT_HOME is not available")?;
    let spritesheet_path = super::builtin_spritesheet_path(reflect_home, pet.spritesheet_file);
    if !spritesheet_path.exists() {
        bail!("missing spritesheet {}", spritesheet_path.display());
    }

    Ok(Pet {
        id: pet.id.to_string(),
        display_name: pet.display_name.to_string(),
        description: Some(pet.description.to_string()),
        spritesheet_path,
        frame_width: catalog::DEFAULT_FRAME_WIDTH,
        frame_height: catalog::DEFAULT_FRAME_HEIGHT,
        columns: catalog::DEFAULT_FRAME_COLUMNS,
        rows: catalog::DEFAULT_FRAME_ROWS,
        frame_count: default_frame_count(),
        animations: default_animations(),
    })
}

fn load_custom_pet(value: &str, reflect_home: Option<&Path>) -> Result<Pet> {
    let reflect_home = reflect_home.context("REFLECT_HOME is not available")?;
    let pet_dir = reflect_home.join("pets").join(value);
    if pet_dir.join("pet.json").is_file() {
        return load_pet_manifest(&pet_dir, "pet.json", value, &custom_pet_cache_id(value));
    }

    let avatar_dir = reflect_home.join("avatars").join(value);
    if avatar_dir.join("avatar.json").is_file() {
        return load_pet_manifest(
            &avatar_dir,
            "avatar.json",
            value,
            &custom_pet_cache_id(value),
        );
    }

    bail!("unknown pet {value}")
}

fn load_pet_path(value: &str) -> Result<Pet> {
    let path = expand_path(value)?;
    let metadata = fs::metadata(&path).with_context(|| format!("pet path {}", path.display()))?;
    let dir = if metadata.is_dir() {
        path
    } else {
        path.parent()
            .context("pet json path has no containing directory")?
            .to_path_buf()
    };
    let pet_dir = dir
        .canonicalize()
        .with_context(|| format!("resolve {}", dir.display()))?;
    let manifest_file = if pet_dir.join("pet.json").is_file() {
        "pet.json"
    } else if pet_dir.join("avatar.json").is_file() {
        "avatar.json"
    } else {
        bail!("missing pet.json or avatar.json in {}", pet_dir.display());
    };
    let fallback_id = pet_dir
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("pet");
    load_pet_manifest(&pet_dir, manifest_file, fallback_id, fallback_id)
}

fn load_pet_manifest(
    pet_dir: &Path,
    manifest_file: &str,
    fallback_id: &str,
    cache_id: &str,
) -> Result<Pet> {
    let config_path = pet_dir.join(manifest_file);
    let raw = fs::read_to_string(&config_path)
        .with_context(|| format!("read {}", config_path.display()))?;
    let file: PetFile =
        serde_json::from_str(&raw).with_context(|| format!("parse {}", config_path.display()))?;

    let manifest_id = file
        .id
        .as_deref()
        .map(str::trim)
        .filter(|id| !id.is_empty());
    let display_name = file
        .display_name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .or(manifest_id)
        .unwrap_or(fallback_id)
        .to_string();
    let pet_id = if cache_id == fallback_id {
        manifest_id.unwrap_or(fallback_id).to_string()
    } else {
        cache_id.to_string()
    };
    let description = file
        .description
        .map(|description| description.trim().to_string())
        .unwrap_or_default();
    let spritesheet_path = resolve_spritesheet_path(
        pet_dir,
        file.spritesheet_path
            .as_deref()
            .map(str::trim)
            .filter(|path| !path.is_empty())
            .unwrap_or("spritesheet.webp"),
    )?;
    if !spritesheet_path.exists() {
        bail!("missing spritesheet {}", spritesheet_path.display());
    }
    let (spritesheet_width, spritesheet_height) =
        validate_app_spritesheet_dimensions(&spritesheet_path)?;

    let frame = file.frame.unwrap_or_default();
    let frame_count = validate_frame_spec(&frame, spritesheet_width, spritesheet_height)?;
    Ok(Pet {
        id: pet_id,
        display_name,
        description: Some(description),
        spritesheet_path,
        frame_width: frame.width,
        frame_height: frame.height,
        columns: frame.columns,
        rows: frame.rows,
        frame_count,
        animations: load_animations(file.animations, frame_count)?,
    })
}

/// 解析清单相对的 sprite 表路径，并保证其留在 pet 目录内。
///
/// 清单格式有意只支持相对子路径。
/// 此处若允许绝对路径或向上的父级穿越路径，
/// 一个自定义 pet 清单就能触及自身目录之外，
/// 并使缓存/调试行为依赖于无关的本地文件。
fn resolve_spritesheet_path(pet_dir: &Path, spritesheet_path: &str) -> Result<PathBuf> {
    let path = Path::new(spritesheet_path);
    if path.is_absolute()
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir | Component::Prefix(_)))
    {
        bail!("spritesheet path must stay inside {}", pet_dir.display());
    }
    Ok(pet_dir.join(path))
}

fn validate_app_spritesheet_dimensions(path: &Path) -> Result<(u32, u32)> {
    let (width, height) =
        image::image_dimensions(path).with_context(|| format!("read {}", path.display()))?;
    if width != catalog::SPRITESHEET_WIDTH || height != catalog::SPRITESHEET_HEIGHT {
        bail!(
            "spritesheet must be {}x{} pixels",
            catalog::SPRITESHEET_WIDTH,
            catalog::SPRITESHEET_HEIGHT
        );
    }
    Ok((width, height))
}

fn validate_frame_spec(
    frame: &FrameSpec,
    spritesheet_width: u32,
    spritesheet_height: u32,
) -> Result<usize> {
    if frame.width == 0 || frame.height == 0 || frame.columns == 0 || frame.rows == 0 {
        bail!("pet frame dimensions and grid counts must be non-zero");
    }

    let total_width = frame
        .width
        .checked_mul(frame.columns)
        .context("pet frame grid width overflow")?;
    let total_height = frame
        .height
        .checked_mul(frame.rows)
        .context("pet frame grid height overflow")?;
    if total_width != spritesheet_width || total_height != spritesheet_height {
        bail!(
            "pet frame grid must cover spritesheet exactly: expected {spritesheet_width}x{spritesheet_height}, got {total_width}x{total_height}"
        );
    }

    let frame_count = frame
        .columns
        .checked_mul(frame.rows)
        .context("pet frame count overflow")?;
    let frame_count = usize::try_from(frame_count).context("pet frame count does not fit usize")?;
    if frame_count > MAX_PET_FRAMES {
        bail!("pet frame count {frame_count} exceeds maximum {MAX_PET_FRAMES}");
    }
    Ok(frame_count)
}

fn custom_pet_cache_id(id: &str) -> String {
    format!("custom-{id}")
}

fn path_like(value: &str) -> bool {
    value == "."
        || value == ".."
        || value.starts_with("~/")
        || value.starts_with("../")
        || value.starts_with("./")
        || Path::new(value).is_absolute()
        || value.contains('/')
        || value.contains('\\')
}

fn expand_path(value: &str) -> Result<PathBuf> {
    if value == "~" || value.starts_with("~/") {
        let home = std::env::var_os("HOME").context("HOME is not set")?;
        if value == "~" {
            return Ok(PathBuf::from(home));
        }
        return Ok(PathBuf::from(home).join(&value[2..]));
    }

    Ok(PathBuf::from(value))
}

fn load_animations(
    specs: HashMap<String, AnimationSpec>,
    frame_count: usize,
) -> Result<HashMap<String, Animation>> {
    let mut animations = default_animations();
    if specs.is_empty() {
        validate_animation_indices(&animations, frame_count)?;
        return Ok(animations);
    }

    for (name, spec) in specs {
        if spec.frames.is_empty() {
            bail!("animation {name} must include at least one frame");
        }
        for sprite_index in &spec.frames {
            if *sprite_index >= frame_count {
                bail!(
                    "animation {name} references sprite index {sprite_index}, but pet has {frame_count} frames"
                );
            }
        }

        let fps = match spec.fps {
            Some(fps) if fps.is_finite() && fps > 0.0 && fps <= MAX_ANIMATION_FPS => fps,
            Some(fps) => {
                bail!(
                    "animation {name} fps must be finite and between 0 and {MAX_ANIMATION_FPS}, got {fps}"
                );
            }
            None => 8.0,
        };
        let duration = Duration::from_secs_f64(1.0 / fps);
        let fallback = if spec.fallback.is_empty() {
            "idle".to_string()
        } else {
            spec.fallback
        };
        let loop_start = spec
            .loop_animation
            .unwrap_or(/*default*/ true)
            .then_some(/*loop_start*/ 0);

        animations.insert(
            name,
            Animation {
                frames: spec
                    .frames
                    .into_iter()
                    .map(|sprite_index| AnimationFrame {
                        sprite_index,
                        duration,
                    })
                    .collect(),
                loop_start,
                fallback,
            },
        );
    }

    animations
        .entry("idle".to_string())
        .or_insert_with(idle_animation);
    validate_animation_indices(&animations, frame_count)?;
    Ok(animations)
}

fn validate_animation_indices(
    animations: &HashMap<String, Animation>,
    frame_count: usize,
) -> Result<()> {
    for (name, animation) in animations {
        if animation.frames.is_empty() {
            bail!("animation {name} must include at least one frame");
        }
        for frame in &animation.frames {
            if frame.sprite_index >= frame_count {
                bail!(
                    "animation {name} references sprite index {}, but pet has {frame_count} frames",
                    frame.sprite_index
                );
            }
        }
        if !animations.contains_key(&animation.fallback) {
            bail!(
                "animation {name} fallback {} does not exist",
                animation.fallback
            );
        }
    }
    Ok(())
}

fn default_frame_count() -> usize {
    (catalog::DEFAULT_FRAME_COLUMNS * catalog::DEFAULT_FRAME_ROWS) as usize
}

fn default_animations() -> HashMap<String, Animation> {
    [
        ("idle", idle_animation()),
        (
            "running-right",
            app_state_animation(
                /*row_index*/ 1, /*frame_count*/ 8, /*frame_duration_ms*/ 120,
                /*final_frame_duration_ms*/ 220,
            ),
        ),
        (
            "running-left",
            app_state_animation(
                /*row_index*/ 2, /*frame_count*/ 8, /*frame_duration_ms*/ 120,
                /*final_frame_duration_ms*/ 220,
            ),
        ),
        (
            "waving",
            app_state_animation(
                /*row_index*/ 3, /*frame_count*/ 4, /*frame_duration_ms*/ 140,
                /*final_frame_duration_ms*/ 280,
            ),
        ),
        (
            "jumping",
            app_state_animation(
                /*row_index*/ 4, /*frame_count*/ 5, /*frame_duration_ms*/ 140,
                /*final_frame_duration_ms*/ 280,
            ),
        ),
        (
            "failed",
            app_state_animation(
                /*row_index*/ 5, /*frame_count*/ 8, /*frame_duration_ms*/ 140,
                /*final_frame_duration_ms*/ 240,
            ),
        ),
        (
            "waiting",
            app_state_animation(
                /*row_index*/ 6, /*frame_count*/ 6, /*frame_duration_ms*/ 150,
                /*final_frame_duration_ms*/ 260,
            ),
        ),
        (
            "running",
            app_state_animation(
                /*row_index*/ 7, /*frame_count*/ 6, /*frame_duration_ms*/ 120,
                /*final_frame_duration_ms*/ 220,
            ),
        ),
        (
            "review",
            app_state_animation(
                /*row_index*/ 8, /*frame_count*/ 6, /*frame_duration_ms*/ 150,
                /*final_frame_duration_ms*/ 280,
            ),
        ),
        (
            "move_right",
            app_state_animation(
                /*row_index*/ 1, /*frame_count*/ 8, /*frame_duration_ms*/ 120,
                /*final_frame_duration_ms*/ 220,
            ),
        ),
        (
            "move_left",
            app_state_animation(
                /*row_index*/ 2, /*frame_count*/ 8, /*frame_duration_ms*/ 120,
                /*final_frame_duration_ms*/ 220,
            ),
        ),
        (
            "wave",
            app_state_animation(
                /*row_index*/ 3, /*frame_count*/ 4, /*frame_duration_ms*/ 140,
                /*final_frame_duration_ms*/ 280,
            ),
        ),
        (
            "bounce",
            app_state_animation(
                /*row_index*/ 4, /*frame_count*/ 5, /*frame_duration_ms*/ 140,
                /*final_frame_duration_ms*/ 280,
            ),
        ),
        (
            "sad",
            app_state_animation(
                /*row_index*/ 5, /*frame_count*/ 8, /*frame_duration_ms*/ 140,
                /*final_frame_duration_ms*/ 240,
            ),
        ),
    ]
    .into_iter()
    .map(|(name, animation)| (name.to_string(), animation))
    .collect()
}

fn idle_animation() -> Animation {
    Animation {
        frames: [(0, 1680), (1, 660), (2, 660), (3, 840), (4, 840), (5, 1920)]
            .into_iter()
            .map(|(sprite_index, duration_ms)| AnimationFrame {
                sprite_index,
                duration: Duration::from_millis(duration_ms),
            })
            .collect(),
        loop_start: Some(/*loop_start*/ 0),
        fallback: "idle".to_string(),
    }
}

fn app_state_animation(
    row_index: usize,
    frame_count: usize,
    frame_duration_ms: u64,
    final_frame_duration_ms: u64,
) -> Animation {
    let primary_frames = (0..frame_count)
        .map(|column_index| AnimationFrame {
            sprite_index: row_index * catalog::DEFAULT_FRAME_COLUMNS as usize + column_index,
            duration: Duration::from_millis(if column_index == frame_count - 1 {
                final_frame_duration_ms
            } else {
                frame_duration_ms
            }),
        })
        .collect::<Vec<_>>();
    let primary_frame_count = primary_frames.len() * 3;
    let frames = primary_frames
        .iter()
        .chain(primary_frames.iter())
        .chain(primary_frames.iter())
        .cloned()
        .chain(idle_animation().frames)
        .collect();
    Animation {
        frames,
        loop_start: Some(primary_frame_count),
        fallback: "idle".to_string(),
    }
}

#[cfg(all(test, feature = "tui-upstream-tests"))]
#[cfg(all(test, feature = "tui-upstream-tests"))]
mod tests;
