//! 通过 /pets 斜杠命令配置的环境终端宠物。
//!
//! TUI 有意区分对待内置和自定义宠物：
//! 内置宠物是按需获取到受管理 REFLECT_HOME 缓存的版本化应用资产，
//! 而自定义宠物是完全归用户所有的数据，位于 `$REFLECT_HOME/pets/<pet-id>/pet.json`
//! 或遗留头像目录下。
//!
//! 本模块持有该区分所涉及的 TUI 侧契约：
//! 解析选中的宠物 id、为终端图像协议准备帧、
//! 渲染环境 sprite 与选择器预览，并保留足够
//! 元数据使 `/pets` 表现得像一等配置界面。
//! 它不负责配置持久化或弹窗编排；调用方必须
//! 在加载前确保内置资产存在，并且只在加载成功后
//! 才持久化最终选择。

use std::io::Write;

mod ambient;
mod asset_pack;
mod catalog;
mod frames;
mod image_protocol;
mod model;
mod picker;
mod preview;
mod sixel;

use anyhow::Context;
use anyhow::Result;

pub(crate) use ambient::AmbientPet;
pub(crate) use ambient::AmbientPetDraw;
pub(crate) use ambient::PetNotificationKind;
#[cfg(test)]
pub(crate) use ambient::test_ambient_pet;
pub(crate) use asset_pack::builtin_spritesheet_path;
#[cfg(test)]
pub(crate) use asset_pack::write_test_pack;
#[cfg(test)]
pub(crate) use image_protocol::ImageProtocol;
pub(crate) use image_protocol::PetImageSupport;
#[cfg(test)]
pub(crate) use image_protocol::PetImageUnsupportedReason;
#[cfg(not(test))]
pub(crate) use image_protocol::detect_pet_image_support;
pub(crate) use picker::PET_PICKER_VIEW_ID;
pub(crate) use picker::build_pet_picker_params;
pub(crate) use preview::PetPickerPreviewState;

pub(crate) const DEFAULT_PET_ID: &str = "reflect";
pub(crate) const DISABLED_PET_ID: &str = "disabled";

/// 确保选中的内置宠物拥有本地缓存的 sprite 表。
///
/// 自定义宠物在此刻意为空操作，因为其数据源本身
/// 就在本地。调用方应在加载内置宠物用于预览或选择前调用它；
/// 跳过此步骤会让首次使用的预览和
/// 持久化失败依赖于更深层的图像加载错误，而不是
/// 资产获取边界。
pub(crate) fn ensure_builtin_pack_for_pet(
    pet_id: &str,
    reflect_home: &std::path::Path,
) -> Result<()> {
    if let Some(pet) = catalog::builtin_pet(pet_id) {
        asset_pack::ensure_builtin_pet(reflect_home, pet)?;
    }
    Ok(())
}

#[derive(Debug)]
pub(crate) enum PetImageRenderError {
    Terminal(std::io::Error),
    Asset(anyhow::Error),
}

impl std::fmt::Display for PetImageRenderError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Terminal(err) => write!(f, "terminal image write failed: {err}"),
            Self::Asset(err) => write!(f, "pet image asset unavailable: {err}"),
        }
    }
}

impl std::error::Error for PetImageRenderError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Terminal(err) => Some(err),
            Self::Asset(err) => Some(err.as_ref()),
        }
    }
}

impl From<std::io::Error> for PetImageRenderError {
    fn from(err: std::io::Error) -> Self {
        Self::Terminal(err)
    }
}

pub(crate) fn render_ambient_pet_image(
    writer: &mut impl Write,
    state: &mut PetImageRenderState,
    request: Option<AmbientPetDraw>,
) -> std::result::Result<(), PetImageRenderError> {
    render_pet_image(writer, state, /*image_id*/ 0xC0DE, request)
}

pub(crate) fn render_pet_picker_preview_image(
    writer: &mut impl Write,
    state: &mut PetImageRenderState,
    request: Option<AmbientPetDraw>,
) -> std::result::Result<(), PetImageRenderError> {
    render_pet_image(writer, state, /*image_id*/ 0xC0DF, request)
}

#[derive(Debug, Default)]
pub(crate) struct PetImageRenderState {
    last_sixel_clear_area: Option<SixelClearArea>,
    last_protocol: Option<image_protocol::ImageProtocol>,
}

fn render_pet_image(
    writer: &mut impl Write,
    state: &mut PetImageRenderState,
    image_id: u32,
    request: Option<AmbientPetDraw>,
) -> std::result::Result<(), PetImageRenderError> {
    use crossterm::cursor::MoveTo;
    use crossterm::cursor::RestorePosition;
    use crossterm::cursor::SavePosition;
    use crossterm::queue;
    use image_protocol::ImageProtocol;

    let Some(request) = request else {
        if state.last_protocol.take().is_some_and(is_kitty_protocol) {
            write!(writer, "{}", image_protocol::kitty_delete_image(image_id))?;
        }
        if let Some(area) = state.last_sixel_clear_area.take() {
            queue!(writer, SavePosition)?;
            clear_sixel_area(writer, area)?;
            queue!(writer, RestorePosition)?;
        }
        writer.flush()?;
        return Ok(());
    };

    if state.last_protocol.take().is_some_and(is_kitty_protocol)
        || is_kitty_protocol(request.protocol)
    {
        write!(writer, "{}", image_protocol::kitty_delete_image(image_id))?;
    }
    state.last_protocol = Some(request.protocol);

    let payload = match request.protocol {
        ImageProtocol::Kitty => AmbientPetPayload::Text(
            image_protocol::kitty_transmit_png_with_id(
                &request.frame,
                request.columns,
                request.rows,
                Some(image_id),
            )
            .map_err(PetImageRenderError::Asset)?,
        ),
        ImageProtocol::KittyLocalFile => AmbientPetPayload::Text(
            image_protocol::kitty_transmit_png_file_with_id(
                &request.frame,
                request.columns,
                request.rows,
                Some(image_id),
            )
            .map_err(PetImageRenderError::Asset)?,
        ),
        ImageProtocol::Sixel => {
            let path =
                image_protocol::sixel_frame(&request.frame, &request.sixel_dir, request.height_px)
                    .map_err(PetImageRenderError::Asset)?;
            let sixel = std::fs::read(&path)
                .with_context(|| format!("read {}", path.display()))
                .map_err(PetImageRenderError::Asset)?;
            AmbientPetPayload::Bytes(sixel)
        }
    };

    queue!(writer, SavePosition)?;
    let current_sixel_clear_area = if matches!(request.protocol, ImageProtocol::Sixel) {
        Some(SixelClearArea::from(&request))
    } else {
        None
    };
    if let Some(previous_area) = state.last_sixel_clear_area.take()
        && Some(previous_area) != current_sixel_clear_area
    {
        clear_sixel_area(writer, previous_area)?;
    }
    if let Some(area) = current_sixel_clear_area {
        clear_sixel_area(writer, area)?;
        state.last_sixel_clear_area = Some(area);
    }
    queue!(writer, MoveTo(request.x, request.y))?;
    match payload {
        AmbientPetPayload::Text(payload) => write!(writer, "{payload}")?,
        AmbientPetPayload::Bytes(payload) => writer.write_all(&payload)?,
    }
    queue!(writer, RestorePosition)?;
    writer.flush()?;
    Ok(())
}

enum AmbientPetPayload {
    Text(String),
    Bytes(Vec<u8>),
}

fn is_kitty_protocol(protocol: image_protocol::ImageProtocol) -> bool {
    matches!(
        protocol,
        image_protocol::ImageProtocol::Kitty | image_protocol::ImageProtocol::KittyLocalFile
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SixelClearArea {
    x: u16,
    clear_top_y: u16,
    clear_bottom_y: u16,
    columns: u16,
}

impl From<&AmbientPetDraw> for SixelClearArea {
    fn from(request: &AmbientPetDraw) -> Self {
        Self {
            x: request.x,
            clear_top_y: request.clear_top_y,
            clear_bottom_y: request.y.saturating_add(request.rows),
            columns: request.columns,
        }
    }
}

fn clear_sixel_area(writer: &mut impl Write, area: SixelClearArea) -> std::io::Result<()> {
    use crossterm::cursor::MoveTo;
    use crossterm::queue;

    let blank = " ".repeat(area.columns.into());
    for row in area.clear_top_y..area.clear_bottom_y {
        queue!(writer, MoveTo(area.x, row))?;
        write!(writer, "{blank}")?;
    }
    Ok(())
}

#[cfg(all(test, feature = "tui-upstream-tests"))]
mod tests {
    use std::error::Error as _;
    use std::io;
    use std::path::PathBuf;

    use super::image_protocol::ImageProtocol;
    use super::*;

    #[test]
    fn ambient_pet_image_restores_cursor_after_drawing() {
        let dir = tempfile::tempdir().unwrap();
        let frame = dir.path().join("frame.png");
        std::fs::write(&frame, b"png").unwrap();
        let request = AmbientPetDraw {
            frame,
            protocol: ImageProtocol::Kitty,
            x: 2,
            y: 3,
            clear_top_y: 3,
            columns: 4,
            rows: 5,
            height_px: 75,
            sixel_dir: PathBuf::new(),
        };
        let mut output = Vec::new();
        let mut state = PetImageRenderState::default();

        render_ambient_pet_image(&mut output, &mut state, Some(request)).unwrap();

        let output = String::from_utf8(output).unwrap();
        let save = output.find("\x1b7").expect("saves cursor position");
        let move_to = output.find("\x1b[4;3H").expect("moves to pet position");
        let image = output.find("cG5n").expect("writes image payload");
        let restore = output.find("\x1b8").expect("restores cursor position");
        assert!(save < move_to);
        assert!(move_to < image);
        assert!(image < restore);
    }

    #[test]
    fn kitty_pet_image_clear_deletes_without_moving_cursor() {
        let dir = tempfile::tempdir().unwrap();
        let frame = dir.path().join("frame.png");
        std::fs::write(&frame, b"png").unwrap();
        let request = AmbientPetDraw {
            frame,
            protocol: ImageProtocol::Kitty,
            x: 2,
            y: 3,
            clear_top_y: 3,
            columns: 4,
            rows: 5,
            height_px: 75,
            sixel_dir: PathBuf::new(),
        };
        let mut output = Vec::new();
        let mut state = PetImageRenderState::default();

        render_ambient_pet_image(&mut output, &mut state, Some(request)).unwrap();
        output.clear();
        render_ambient_pet_image(&mut output, &mut state, /*request*/ None).unwrap();

        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("Ga=d,d=I,i=49374,q=2;"));
        assert!(!output.contains("\x1b7"));
        assert!(!output.contains("\x1b["));
        assert!(!output.contains("\x1b8"));
    }

    #[test]
    fn kitty_local_file_pet_image_uses_file_reference_without_inline_payload() {
        let dir = tempfile::tempdir().unwrap();
        let frame = dir.path().join("frame.png");
        std::fs::write(&frame, b"png").unwrap();
        let request = AmbientPetDraw {
            frame,
            protocol: ImageProtocol::KittyLocalFile,
            x: 2,
            y: 3,
            clear_top_y: 3,
            columns: 4,
            rows: 2,
            height_px: 75,
            sixel_dir: PathBuf::new(),
        };
        let mut output = Vec::new();
        let mut state = PetImageRenderState::default();

        render_ambient_pet_image(&mut output, &mut state, Some(request)).unwrap();

        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("a=d,d=I,i=49374,q=2;"));
        assert!(output.contains("\x1b[4;3H"));
        assert!(output.contains("a=T,t=f,f=100,c=4,r=2,q=2,i=49374;"));
        assert!(!output.contains("cG5n"));
        assert!(output.contains("\x1b8"));
    }

    #[test]
    fn sixel_pet_image_clears_cell_area_before_redrawing() {
        let dir = tempfile::tempdir().unwrap();
        let frame = dir.path().join("frame.png");
        std::fs::write(&frame, b"png").unwrap();
        let sixel_dir = dir.path().join("sixel");
        std::fs::create_dir(&sixel_dir).unwrap();
        let sixel_frame = sixel_dir.join("frame_h75_v2.six");
        std::fs::write(&sixel_frame, b"fake-sixel").unwrap();
        let request = AmbientPetDraw {
            frame,
            protocol: ImageProtocol::Sixel,
            x: 2,
            y: 3,
            clear_top_y: 1,
            columns: 4,
            rows: 2,
            height_px: 75,
            sixel_dir,
        };
        let mut output = Vec::new();
        let mut state = PetImageRenderState::default();

        render_ambient_pet_image(&mut output, &mut state, Some(request)).unwrap();

        let output = String::from_utf8(output).unwrap();
        assert!(output.contains("\x1b[2;3H    \x1b[3;3H    \x1b[4;3H    \x1b[5;3H    \x1b[4;3H"));
        assert!(output.contains("fake-sixel"));
        assert!(output.contains("\x1b8"));
    }

    #[test]
    fn sixel_pet_image_clear_erases_last_drawn_area() {
        let dir = tempfile::tempdir().unwrap();
        let frame = dir.path().join("frame.png");
        std::fs::write(&frame, b"png").unwrap();
        let sixel_dir = dir.path().join("sixel");
        std::fs::create_dir(&sixel_dir).unwrap();
        let sixel_frame = sixel_dir.join("frame_h75_v2.six");
        std::fs::write(&sixel_frame, b"fake-sixel").unwrap();
        let request = AmbientPetDraw {
            frame,
            protocol: ImageProtocol::Sixel,
            x: 2,
            y: 3,
            clear_top_y: 1,
            columns: 4,
            rows: 2,
            height_px: 75,
            sixel_dir,
        };
        let mut output = Vec::new();
        let mut state = PetImageRenderState::default();

        render_ambient_pet_image(&mut output, &mut state, Some(request)).unwrap();
        output.clear();
        render_ambient_pet_image(&mut output, &mut state, /*request*/ None).unwrap();

        let output = String::from_utf8(output).unwrap();
        assert!(!output.contains("Ga=d,d=I,i=49374,q=2;"));
        assert!(output.contains("\x1b7"));
        assert!(output.contains("\x1b[2;3H    \x1b[3;3H    \x1b[4;3H    \x1b[5;3H    "));
        assert!(output.contains("\x1b8"));
        assert!(!output.contains("fake-sixel"));
    }

    #[test]
    fn missing_frame_is_an_asset_error() {
        let dir = tempfile::tempdir().unwrap();
        let request = AmbientPetDraw {
            frame: dir.path().join("missing.png"),
            protocol: ImageProtocol::Kitty,
            x: 2,
            y: 3,
            clear_top_y: 3,
            columns: 4,
            rows: 5,
            height_px: 75,
            sixel_dir: PathBuf::new(),
        };
        let mut output = Vec::new();
        let mut state = PetImageRenderState::default();

        let err = render_ambient_pet_image(&mut output, &mut state, Some(request)).unwrap_err();

        assert!(matches!(err, PetImageRenderError::Asset(_)));
        assert!(err.source().is_some());
    }

    #[test]
    fn writer_failure_is_a_terminal_error() {
        struct FailingWriter;

        impl io::Write for FailingWriter {
            fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
                Err(io::Error::new(
                    io::ErrorKind::BrokenPipe,
                    "test writer failed",
                ))
            }

            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        let mut writer = FailingWriter;
        let mut state = PetImageRenderState {
            last_protocol: Some(ImageProtocol::Kitty),
            ..Default::default()
        };

        let err = render_ambient_pet_image(&mut writer, &mut state, /*request*/ None).unwrap_err();

        assert!(matches!(err, PetImageRenderError::Terminal(_)));
        assert!(err.source().is_some());
    }
}
