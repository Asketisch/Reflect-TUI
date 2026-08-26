//! 宠物模型（pets/model） 的测试集。
//!
//! 从 pets/model.rs 的内联 mod tests 块外移而来，业务逻辑零改动，
//! 仅做结构性拆分以收敛单文件行数（遵循 CLAUDE.md 文件行数规范）。

use super::*;

fn write_minimal_pet() -> tempfile::TempDir {
    write_pet_manifest(
        r#"{
                "id": "chefito",
                "displayName": "Chefito",
                "description": "A tiny recipe-loving chef",
                "spritesheetPath": "spritesheet.webp"
            }"#,
    )
}

fn write_pet_manifest(manifest: &str) -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("pet.json"), manifest).unwrap();
    catalog::write_test_spritesheet(&dir.path().join("spritesheet.webp"));
    dir
}

fn load_pet_from_dir(dir: &tempfile::TempDir) -> Pet {
    Pet::load_with_reflect_home(dir.path().to_str().unwrap(), /*reflect_home*/ None).unwrap()
}

fn load_pet_error_from_dir(dir: &tempfile::TempDir) -> anyhow::Error {
    Pet::load_with_reflect_home(dir.path().to_str().unwrap(), /*reflect_home*/ None).unwrap_err()
}

#[test]
fn load_builtin_pet_uses_app_catalog_storage() {
    let reflect_home = tempfile::tempdir().unwrap();
    super::super::asset_pack::write_test_pack(reflect_home.path());

    let pet =
        Pet::load_with_reflect_home("dewey", /*reflect_home*/ Some(reflect_home.path())).unwrap();

    assert_eq!(pet.id, "dewey");
    assert_eq!(pet.display_name, "Dewey");
    assert_eq!(pet.description, "A tidy duck for calm workspace days");
    assert_eq!(
        pet.spritesheet_path,
        super::super::builtin_spritesheet_path(reflect_home.path(), "dewey-spritesheet-v4.webp")
    );
    assert_eq!(pet.frame_width, 192);
    assert_eq!(pet.frame_height, 208);
    assert_eq!(pet.columns, 8);
    assert_eq!(pet.rows, 9);
}

#[test]
fn app_idle_animation_uses_calm_loop() {
    let animations = default_animations();
    let idle = &animations["idle"];

    assert_eq!(sprite_indices(idle), vec![0, 1, 2, 3, 4, 5]);
    assert_eq!(durations_ms(idle), vec![1680, 660, 660, 840, 840, 1920]);
    assert_eq!(idle.loop_start, Some(/*loop_start*/ 0));
}

#[test]
fn app_running_animation_repeats_then_settles_into_idle() {
    let animations = default_animations();
    let running = &animations["running"];
    let primary = vec![56, 57, 58, 59, 60, 61];

    assert_eq!(sprite_indices(running)[0..6], primary);
    assert_eq!(sprite_indices(running)[6..12], primary);
    assert_eq!(sprite_indices(running)[12..18], primary);
    assert_eq!(
        sprite_indices(running)[18..],
        sprite_indices(&animations["idle"])
    );
    assert_eq!(
        durations_ms(running)[0..6],
        vec![120, 120, 120, 120, 120, 220]
    );
    assert_eq!(running.loop_start, Some(/*loop_start*/ 18));
}

#[test]
fn app_notification_states_use_expected_rows() {
    let animations = default_animations();

    assert_eq!(
        sprite_indices(&animations["waiting"])[0..6],
        vec![48, 49, 50, 51, 52, 53]
    );
    assert_eq!(
        sprite_indices(&animations["review"])[0..6],
        vec![64, 65, 66, 67, 68, 69]
    );
    assert_eq!(
        sprite_indices(&animations["failed"])[0..8],
        vec![40, 41, 42, 43, 44, 45, 46, 47]
    );
}

#[test]
fn custom_animation_specs_keep_manifest_fps_and_loop_shape() {
    let animations = load_animations(
        HashMap::from([(
            "custom".to_string(),
            AnimationSpec {
                frames: vec![1, 2],
                fps: Some(/*fps*/ 2.0),
                loop_animation: Some(/*loop_animation*/ false),
                fallback: "idle".to_string(),
            },
        )]),
        default_frame_count(),
    )
    .unwrap();
    let custom = &animations["custom"];

    assert_eq!(sprite_indices(custom), vec![1, 2]);
    assert_eq!(durations_ms(custom), vec![500, 500]);
    assert_eq!(custom.loop_start, None);
    assert_eq!(custom.fallback, "idle");
}

#[test]
fn load_pet_directory_uses_app_pet_manifest_defaults() {
    let dir = write_minimal_pet();

    let pet = load_pet_from_dir(&dir);

    assert_eq!(pet.id, "chefito");
    assert_eq!(pet.display_name, "Chefito");
    assert_eq!(pet.frame_width, 192);
    assert_eq!(pet.frame_height, 208);
    assert_eq!(pet.columns, 8);
    assert_eq!(pet.rows, 9);
    assert_eq!(pet.frame_count(), 72);
    assert!(!pet.animations["idle"].frames.is_empty());
}

#[test]
fn frame_cache_key_changes_with_spritesheet_contents() {
    let dir = write_minimal_pet();
    let spritesheet_path = dir.path().join("spritesheet.webp");
    let pet = load_pet_from_dir(&dir);
    let first_key = pet.frame_cache_key().unwrap();

    let image = image::RgbaImage::from_pixel(
        catalog::SPRITESHEET_WIDTH,
        catalog::SPRITESHEET_HEIGHT,
        image::Rgba([1, 2, 3, 255]),
    );
    image.save(&spritesheet_path).unwrap();
    let pet = load_pet_from_dir(&dir);

    assert_ne!(pet.frame_cache_key().unwrap(), first_key);
}

#[test]
fn frame_cache_key_changes_with_frame_spec() {
    let default_dir = write_minimal_pet();
    let default_pet = load_pet_from_dir(&default_dir);
    let custom_dir = write_pet_manifest(
        r#"{
                "displayName": "Tall",
                "spritesheetPath": "spritesheet.webp",
                "frame": { "width": 384, "height": 104, "columns": 4, "rows": 18 }
            }"#,
    );
    let custom_pet = load_pet_from_dir(&custom_dir);

    assert_ne!(
        custom_pet.frame_cache_key().unwrap(),
        default_pet.frame_cache_key().unwrap()
    );
}

#[test]
fn load_pet_json_path_uses_containing_directory() {
    let dir = write_minimal_pet();

    let pet = Pet::load_with_reflect_home(
        dir.path().join("pet.json").to_str().unwrap(),
        /*reflect_home*/ None,
    )
    .unwrap();
    let expected = dir.path().join("spritesheet.webp").canonicalize().unwrap();

    assert_eq!(pet.spritesheet_path, expected);
}

#[test]
fn custom_pet_selector_loads_reflect_home_pet_manifest() {
    let dir = write_minimal_pet();
    let reflect_home = tempfile::tempdir().unwrap();
    let pet_dir = reflect_home.path().join("pets").join("chefito");
    fs::create_dir_all(&pet_dir).unwrap();
    fs::copy(dir.path().join("pet.json"), pet_dir.join("pet.json")).unwrap();
    fs::copy(
        dir.path().join("spritesheet.webp"),
        pet_dir.join("spritesheet.webp"),
    )
    .unwrap();

    let pet = Pet::load_with_reflect_home(
        &custom_pet_selector("chefito"),
        /*reflect_home*/ Some(reflect_home.path()),
    )
    .unwrap();

    assert_eq!(pet.id, "custom-chefito");
    assert_eq!(pet.spritesheet_path, pet_dir.join("spritesheet.webp"),);
}

#[test]
fn custom_pet_selector_falls_back_to_legacy_avatar_manifest() {
    let dir = write_minimal_pet();
    let reflect_home = tempfile::tempdir().unwrap();
    let avatar_dir = reflect_home.path().join("avatars").join("legacy");
    fs::create_dir_all(&avatar_dir).unwrap();
    fs::copy(dir.path().join("pet.json"), avatar_dir.join("avatar.json")).unwrap();
    fs::copy(
        dir.path().join("spritesheet.webp"),
        avatar_dir.join("spritesheet.webp"),
    )
    .unwrap();

    let pet = Pet::load_with_reflect_home(
        &custom_pet_selector("legacy"),
        /*reflect_home*/ Some(reflect_home.path()),
    )
    .unwrap();

    assert_eq!(pet.id, "custom-legacy");
    assert_eq!(pet.display_name, "Chefito");
}

#[test]
fn custom_pet_rejects_spritesheet_path_escape() {
    let reflect_home = tempfile::tempdir().unwrap();
    let pet_dir = reflect_home.path().join("pets").join("escape");
    fs::create_dir_all(&pet_dir).unwrap();
    fs::write(
        pet_dir.join("pet.json"),
        r#"{
                "displayName": "Escape",
                "spritesheetPath": "../spritesheet.webp"
            }"#,
    )
    .unwrap();

    let err = Pet::load_with_reflect_home(
        &custom_pet_selector("escape"),
        /*reflect_home*/ Some(reflect_home.path()),
    )
    .unwrap_err();

    assert!(
        err.to_string()
            .contains("spritesheet path must stay inside")
    );
}

#[test]
fn custom_pet_rejects_zero_frame_dimensions() {
    let dir = write_pet_manifest(
        r#"{
                "displayName": "Zero",
                "spritesheetPath": "spritesheet.webp",
                "frame": { "width": 0, "height": 208, "columns": 8, "rows": 9 }
            }"#,
    );

    let err = load_pet_error_from_dir(&dir);

    assert!(
        err.to_string()
            .contains("pet frame dimensions and grid counts must be non-zero")
    );
}

#[test]
fn custom_pet_rejects_frame_grid_that_does_not_cover_spritesheet() {
    let dir = write_pet_manifest(
        r#"{
                "displayName": "Short",
                "spritesheetPath": "spritesheet.webp",
                "frame": { "width": 192, "height": 208, "columns": 7, "rows": 9 }
            }"#,
    );

    let err = load_pet_error_from_dir(&dir);

    assert!(
        err.to_string()
            .contains("pet frame grid must cover spritesheet exactly")
    );
}

#[test]
fn custom_pet_rejects_excessive_frame_count() {
    let dir = write_pet_manifest(
        r#"{
                "displayName": "Dense",
                "spritesheetPath": "spritesheet.webp",
                "frame": { "width": 8, "height": 8, "columns": 192, "rows": 234 }
            }"#,
    );

    let err = load_pet_error_from_dir(&dir);

    assert!(err.to_string().contains("exceeds maximum"));
}

#[test]
fn custom_pet_rejects_empty_animation_frames() {
    let dir = write_pet_manifest(
        r#"{
                "displayName": "Empty",
                "spritesheetPath": "spritesheet.webp",
                "animations": {
                    "idle": { "frames": [] }
                }
            }"#,
    );

    let err = load_pet_error_from_dir(&dir);

    assert!(
        err.to_string()
            .contains("animation idle must include at least one frame")
    );
}

#[test]
fn custom_pet_rejects_animation_frame_outside_grid() {
    let dir = write_pet_manifest(
        r#"{
                "displayName": "Outside",
                "spritesheetPath": "spritesheet.webp",
                "animations": {
                    "idle": { "frames": [72] }
                }
            }"#,
    );

    let err = load_pet_error_from_dir(&dir);

    assert!(
        err.to_string()
            .contains("animation idle references sprite index 72")
    );
}

#[test]
fn custom_pet_rejects_invalid_animation_fps() {
    let dir = write_pet_manifest(
        r#"{
                "displayName": "Fast",
                "spritesheetPath": "spritesheet.webp",
                "animations": {
                    "idle": { "frames": [0], "fps": 120.0 }
                }
            }"#,
    );

    let err = load_pet_error_from_dir(&dir);

    assert!(
        err.to_string()
            .contains("animation idle fps must be finite and between")
    );
}

#[test]
fn custom_pet_rejects_animation_fallback_to_missing_animation() {
    let dir = write_pet_manifest(
        r#"{
                "displayName": "Fallback",
                "spritesheetPath": "spritesheet.webp",
                "animations": {
                    "wave": { "frames": [1], "loop": false, "fallback": "missing" }
                }
            }"#,
    );

    let err = load_pet_error_from_dir(&dir);

    assert!(
        err.to_string()
            .contains("animation wave fallback missing does not exist")
    );
}

fn sprite_indices(animation: &Animation) -> Vec<usize> {
    animation
        .frames
        .iter()
        .map(|frame| frame.sprite_index)
        .collect()
}

fn durations_ms(animation: &Animation) -> Vec<u128> {
    animation
        .frames
        .iter()
        .map(|frame| frame.duration.as_millis())
        .collect()
}
