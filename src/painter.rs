use crate::app::SwitchAppsState;
use crate::utils::{get_monitor_rect_and_dpi, is_light_theme, is_win11, PerfSpan};

use anyhow::{anyhow, Context, Result};
use windows::core::{w, PCWSTR};
use windows::Win32::{
    Foundation::{COLORREF, HWND, POINT, RECT, SIZE},
    Graphics::{
        Gdi::{
            CreateCompatibleBitmap, CreateCompatibleDC, CreateRoundRectRgn, CreateSolidBrush,
            DeleteDC, DeleteObject, FillRect, FillRgn, GetDC, ReleaseDC, SelectObject,
            SetStretchBltMode, StretchBlt, AC_SRC_ALPHA, AC_SRC_OVER, BLENDFUNCTION, HALFTONE,
            HBITMAP, HDC, HPALETTE, SRCCOPY,
        },
        GdiPlus::{
            FillModeAlternate, FontStyleRegular, GdipAddPathArc, GdipClosePathFigure,
            GdipCreateBitmapFromHBITMAP, GdipCreateFont, GdipCreateFontFamilyFromName,
            GdipCreateFromHDC, GdipCreatePath, GdipCreateSolidFill, GdipCreateStringFormat,
            GdipDeleteBrush, GdipDeleteFont, GdipDeleteFontFamily, GdipDeleteGraphics,
            GdipDeletePath, GdipDeleteStringFormat, GdipDisposeImage, GdipDrawImageRect,
            GdipDrawString, GdipFillPath, GdipFillRectangle, GdipSetInterpolationMode,
            GdipSetSmoothingMode, GdipSetStringFormatAlign, GdipSetStringFormatLineAlign,
            GdipSetStringFormatTrimming, GdipSetTextRenderingHint, GdiplusShutdown, GdiplusStartup,
            GdiplusStartupInput, GpBitmap, GpBrush, GpFont, GpFontFamily, GpGraphics, GpImage,
            GpPath, GpSolidFill, GpStringFormat, InterpolationModeHighQualityBicubic, RectF,
            SmoothingModeAntiAlias, Status, StringAlignmentCenter, StringFormatFlagsNoWrap,
            StringTrimmingEllipsisCharacter, TextRenderingHintAntiAliasGridFit, UnitPixel,
        },
    },
    UI::{
        Input::KeyboardAndMouse::SetFocus,
        WindowsAndMessaging::{
            DrawIconEx, GetCursorPos, ShowWindow, UpdateLayeredWindow, DI_NORMAL, SW_HIDE, SW_SHOW,
            ULW_ALPHA,
        },
    },
};

pub const BG_DARK_COLOR: u32 = 0x4c4c4c;
pub const FG_DARK_COLOR: u32 = 0x3b3b3b;
pub const BG_LIGHT_COLOR: u32 = 0xe0e0e0;
pub const FG_LIGHT_COLOR: u32 = 0xf2f2f2;
pub const ALPHA_MASK: u32 = 0xff000000;
pub const ICON_SIZE_BASE: i32 = 64;
pub const WINDOW_BORDER_SIZE_BASE: i32 = 10;
pub const ICON_BORDER_SIZE_BASE: i32 = 4;
pub const TITLE_HEIGHT_BASE: i32 = 26;
pub const TITLE_GAP_BASE: i32 = 4;
pub const TITLE_MIN_WIDTH_BASE: i32 = 320;
pub const TITLE_FONT_SIZE_BASE: i32 = 14;
pub const SCALE_FACTOR: i32 = 2;
pub const TEXT_DARK_COLOR: u32 = 0xf4f4f4;
pub const TEXT_LIGHT_COLOR: u32 = 0x202020;

struct GdiTitleRenderer {
    dpi: u32,
    font_family: *mut GpFontFamily,
    font: *mut GpFont,
    format: *mut GpStringFormat,
    dark_theme_brush: *mut GpSolidFill,
    light_theme_brush: *mut GpSolidFill,
}

impl GdiTitleRenderer {
    fn new(dpi: u32) -> Result<Self> {
        let mut renderer = Self {
            dpi,
            font_family: std::ptr::null_mut(),
            font: std::ptr::null_mut(),
            format: std::ptr::null_mut(),
            dark_theme_brush: std::ptr::null_mut(),
            light_theme_brush: std::ptr::null_mut(),
        };
        let font_size = TITLE_FONT_SIZE_BASE as f32 * dpi as f32 / 96.0;

        unsafe {
            check_gdiplus(GdipCreateFontFamilyFromName(
                w!("Segoe UI Semibold"),
                std::ptr::null_mut(),
                &mut renderer.font_family,
            ))
            .context("Failed to create the title font family")?;
            check_gdiplus(GdipCreateFont(
                renderer.font_family,
                font_size,
                FontStyleRegular.0,
                UnitPixel,
                &mut renderer.font,
            ))
            .context("Failed to create the title font")?;
            check_gdiplus(GdipCreateStringFormat(
                StringFormatFlagsNoWrap.0,
                0,
                &mut renderer.format,
            ))
            .context("Failed to create the title format")?;
            check_gdiplus(GdipSetStringFormatAlign(
                renderer.format,
                StringAlignmentCenter,
            ))
            .context("Failed to center the title")?;
            check_gdiplus(GdipSetStringFormatLineAlign(
                renderer.format,
                StringAlignmentCenter,
            ))
            .context("Failed to vertically center the title")?;
            check_gdiplus(GdipSetStringFormatTrimming(
                renderer.format,
                StringTrimmingEllipsisCharacter,
            ))
            .context("Failed to configure title ellipsis")?;
            check_gdiplus(GdipCreateSolidFill(
                ALPHA_MASK | TEXT_DARK_COLOR,
                &mut renderer.dark_theme_brush,
            ))
            .context("Failed to create the dark-theme title brush")?;
            check_gdiplus(GdipCreateSolidFill(
                ALPHA_MASK | TEXT_LIGHT_COLOR,
                &mut renderer.light_theme_brush,
            ))
            .context("Failed to create the light-theme title brush")?;
        }

        Ok(renderer)
    }

    unsafe fn draw(
        &self,
        graphics: *mut GpGraphics,
        state: &SwitchAppsState,
        rect: RectF,
        text_color: u32,
    ) {
        let Some(window) = state.windows.get(state.index) else {
            return;
        };
        let title = &window.title;
        if title.is_empty() {
            return;
        }

        let text: Vec<u16> = title.encode_utf16().collect();
        let brush = if text_color == TEXT_DARK_COLOR {
            self.dark_theme_brush
        } else {
            self.light_theme_brush
        };

        let _ = GdipSetTextRenderingHint(graphics, TextRenderingHintAntiAliasGridFit);
        let _ = GdipDrawString(
            graphics,
            PCWSTR(text.as_ptr()),
            text.len() as i32,
            self.font,
            &rect,
            self.format,
            brush as *const GpBrush,
        );
    }
}

impl Drop for GdiTitleRenderer {
    fn drop(&mut self) {
        unsafe {
            if !self.dark_theme_brush.is_null() {
                let _ = GdipDeleteBrush(self.dark_theme_brush as *mut GpBrush);
            }
            if !self.light_theme_brush.is_null() {
                let _ = GdipDeleteBrush(self.light_theme_brush as *mut GpBrush);
            }
            if !self.format.is_null() {
                let _ = GdipDeleteStringFormat(self.format);
            }
            if !self.font.is_null() {
                let _ = GdipDeleteFont(self.font);
            }
            if !self.font_family.is_null() {
                let _ = GdipDeleteFontFamily(self.font_family);
            }
        }
    }
}

struct CachedGdiImage {
    bitmap: HBITMAP,
    image: *mut GpImage,
}

impl CachedGdiImage {
    unsafe fn new(bitmap: HBITMAP) -> Self {
        let mut bitmap_ptr: *mut GpBitmap = std::ptr::null_mut();
        let _ = GdipCreateBitmapFromHBITMAP(bitmap, HPALETTE::default(), &mut bitmap_ptr);
        Self {
            bitmap,
            image: bitmap_ptr as *mut GpImage,
        }
    }
}

impl Drop for CachedGdiImage {
    fn drop(&mut self) {
        unsafe {
            if !self.image.is_null() {
                let _ = GdipDisposeImage(self.image);
            }
            if !self.bitmap.is_invalid() {
                let _ = DeleteObject(self.bitmap.into());
            }
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct PaintSpec {
    coordinate: Coordinate,
    border_size: i32,
    icon_border: i32,
    title_height: i32,
    corner_radius: i32,
    fg_color: u32,
    bg_color: u32,
}

struct PaintSession {
    spec: PaintSpec,
    hdc: HDC,
    bitmap: HBITMAP,
    old_bitmap: windows::Win32::Graphics::Gdi::HGDIOBJ,
    graphics: *mut GpGraphics,
    bg_brush: *mut GpSolidFill,
    base_icons: CachedGdiImage,
    selected_icons: Vec<Option<CachedGdiImage>>,
}

impl PaintSession {
    unsafe fn new(state: &SwitchAppsState, hdc_screen: HDC, spec: PaintSpec) -> Self {
        let _perf = PerfSpan::new("create_paint_session");
        let coordinate = spec.coordinate;
        let hdc = CreateCompatibleDC(Some(hdc_screen));
        let bitmap = CreateCompatibleBitmap(hdc_screen, coordinate.width, coordinate.height);
        let old_bitmap = SelectObject(hdc, bitmap.into());

        let mut graphics: *mut GpGraphics = std::ptr::null_mut();
        let _ = GdipCreateFromHDC(hdc, &mut graphics);
        let _ = GdipSetSmoothingMode(graphics, SmoothingModeAntiAlias);
        let _ = GdipSetInterpolationMode(graphics, InterpolationModeHighQualityBicubic);

        let mut bg_brush: *mut GpSolidFill = std::ptr::null_mut();
        let _ = GdipCreateSolidFill(ALPHA_MASK | spec.bg_color, &mut bg_brush);
        if spec.corner_radius > 0 {
            draw_round_rect(
                graphics,
                bg_brush as *mut GpBrush,
                0.0,
                0.0,
                coordinate.width as f32,
                coordinate.height as f32,
                spec.corner_radius as f32,
            );
        } else {
            let _ = GdipFillRectangle(
                graphics,
                bg_brush as *mut GpBrush,
                0.0,
                0.0,
                coordinate.width as f32,
                coordinate.height as f32,
            );
        }

        let icons_width = coordinate.item_size * state.windows.len() as i32;
        let base_bitmap = draw_icons(
            state,
            hdc_screen,
            coordinate.icon_size,
            spec.icon_border,
            icons_width,
            coordinate.item_size,
            spec.bg_color,
        );
        let base_icons = CachedGdiImage::new(base_bitmap);
        let selected_icons = (0..state.windows.len()).map(|_| None).collect();

        Self {
            spec,
            hdc,
            bitmap,
            old_bitmap,
            graphics,
            bg_brush,
            base_icons,
            selected_icons,
        }
    }

    unsafe fn paint(
        &mut self,
        state: &SwitchAppsState,
        hdc_screen: HDC,
        hwnd: HWND,
        title_renderer: &GdiTitleRenderer,
        text_color: u32,
    ) {
        let coordinate = self.spec.coordinate;
        let icons_width = coordinate.item_size * state.windows.len() as i32;
        let _ = GdipDrawImageRect(
            self.graphics,
            self.base_icons.image,
            coordinate.icons_x as f32,
            self.spec.border_size as f32,
            icons_width as f32,
            coordinate.item_size as f32,
        );

        if self.selected_icons[state.index].is_none() {
            let bitmap = draw_selected_icon(
                state.windows[state.index].icon,
                hdc_screen,
                coordinate.icon_size,
                self.spec.icon_border,
                coordinate.item_size,
                self.spec.corner_radius,
                self.spec.fg_color,
                self.spec.bg_color,
            );
            self.selected_icons[state.index] = Some(CachedGdiImage::new(bitmap));
        }
        if let Some(selected) = &self.selected_icons[state.index] {
            let _ = GdipDrawImageRect(
                self.graphics,
                selected.image,
                (coordinate.icons_x + coordinate.item_size * state.index as i32) as f32,
                self.spec.border_size as f32,
                coordinate.item_size as f32,
                coordinate.item_size as f32,
            );
        }

        let _ = GdipFillRectangle(
            self.graphics,
            self.bg_brush as *mut GpBrush,
            self.spec.border_size as f32,
            coordinate.title_top as f32,
            (coordinate.width - self.spec.border_size * 2) as f32,
            self.spec.title_height as f32,
        );
        title_renderer.draw(
            self.graphics,
            state,
            RectF {
                X: self.spec.border_size as f32,
                Y: coordinate.title_top as f32,
                Width: (coordinate.width - self.spec.border_size * 2) as f32,
                Height: self.spec.title_height as f32,
            },
            text_color,
        );

        let blend = BLENDFUNCTION {
            BlendOp: AC_SRC_OVER as _,
            SourceConstantAlpha: 255,
            AlphaFormat: AC_SRC_ALPHA as _,
            ..Default::default()
        };
        let _ = UpdateLayeredWindow(
            hwnd,
            Some(hdc_screen),
            Some(&POINT {
                x: coordinate.x,
                y: coordinate.y,
            }),
            Some(&SIZE {
                cx: coordinate.width,
                cy: coordinate.height,
            }),
            Some(self.hdc),
            Some(&POINT::default()),
            COLORREF(0),
            Some(&blend),
            ULW_ALPHA,
        );
    }
}

impl Drop for PaintSession {
    fn drop(&mut self) {
        unsafe {
            if !self.graphics.is_null() {
                let _ = GdipDeleteGraphics(self.graphics);
            }
            if !self.bg_brush.is_null() {
                let _ = GdipDeleteBrush(self.bg_brush as *mut GpBrush);
            }
            SelectObject(self.hdc, self.old_bitmap);
            let _ = DeleteObject(self.bitmap.into());
            let _ = DeleteDC(self.hdc);
        }
    }
}

// GDI Antialiasing Painter
pub struct GdiAAPainter {
    token: usize,
    hwnd: HWND,
    hdc_screen: HDC,
    title_renderer: Option<GdiTitleRenderer>,
    session: Option<PaintSession>,
    light_theme: bool,
    rounded_corner: bool,
    show: bool,
}

fn check_gdiplus(status: Status) -> Result<()> {
    if status.0 == 0 {
        Ok(())
    } else {
        Err(anyhow!("GDI+ operation failed with status {}", status.0))
    }
}

impl GdiAAPainter {
    pub fn new(hwnd: HWND) -> Result<Self> {
        let startup_input = GdiplusStartupInput {
            GdiplusVersion: 1,
            ..Default::default()
        };
        let mut token: usize = 0;
        check_gdiplus(unsafe { GdiplusStartup(&mut token, &startup_input, std::ptr::null_mut()) })
            .context("Failed to initialize GDI+")?;

        let (_, dpi) = get_monitor_rect_and_dpi();
        let title_renderer = match GdiTitleRenderer::new(dpi) {
            Ok(renderer) => renderer,
            Err(error) => {
                unsafe { GdiplusShutdown(token) };
                return Err(error);
            }
        };

        let hdc_screen = unsafe { GetDC(Some(hwnd)) };
        let rounded_corner = is_win11();

        Ok(Self {
            token,
            hwnd,
            hdc_screen,
            title_renderer: Some(title_renderer),
            session: None,
            light_theme: is_light_theme(),
            rounded_corner,
            show: false,
        })
    }

    pub fn refresh_theme(&mut self) -> bool {
        let light_theme = is_light_theme();
        let changed = light_theme != self.light_theme;
        self.light_theme = light_theme;
        if changed {
            drop(self.session.take());
        }
        changed
    }

    pub fn invalidate_session(&mut self) {
        drop(self.session.take());
    }

    pub fn paint(&mut self, state: &SwitchAppsState) {
        let _perf = PerfSpan::new("paint_switcher");
        if state.windows.is_empty() {
            return;
        }
        let (monitor_rect, dpi) = get_monitor_rect_and_dpi();
        let title_dpi_changed = self
            .title_renderer
            .as_ref()
            .is_none_or(|renderer| renderer.dpi != dpi);
        if title_dpi_changed {
            match GdiTitleRenderer::new(dpi) {
                Ok(renderer) => {
                    drop(self.title_renderer.replace(renderer));
                    self.invalidate_session();
                }
                Err(error) => error!("failed to update title DPI: {error}"),
            }
        }
        let dpi_scale = dpi as f64 / 96.0;
        let icon_size_max = (ICON_SIZE_BASE as f64 * dpi_scale) as i32;
        let border_size = (WINDOW_BORDER_SIZE_BASE as f64 * dpi_scale) as i32;
        let icon_border = (ICON_BORDER_SIZE_BASE as f64 * dpi_scale) as i32;
        let title_height = (TITLE_HEIGHT_BASE as f64 * dpi_scale) as i32;
        let title_gap = (TITLE_GAP_BASE as f64 * dpi_scale) as i32;
        let title_min_width = (TITLE_MIN_WIDTH_BASE as f64 * dpi_scale) as i32;
        let metrics = LayoutMetrics {
            icon_size_max,
            border_size,
            icon_border,
            title_height,
            title_gap,
            title_min_width,
        };

        let coordinate = Coordinate::new(state.windows.len() as i32, metrics, monitor_rect);

        let corner_radius = if self.rounded_corner {
            coordinate.item_size / 4
        } else {
            0
        };
        let (fg_color, bg_color, text_color) = theme_color(self.light_theme);
        let spec = PaintSpec {
            coordinate,
            border_size,
            icon_border,
            title_height,
            corner_radius,
            fg_color,
            bg_color,
        };
        let recreate = match &self.session {
            Some(session) => session.spec != spec,
            None => true,
        };
        if recreate {
            drop(self.session.take());
            self.session = Some(unsafe { PaintSession::new(state, self.hdc_screen, spec) });
        }
        if let (Some(session), Some(title_renderer)) =
            (self.session.as_mut(), self.title_renderer.as_ref())
        {
            unsafe {
                session.paint(
                    state,
                    self.hdc_screen,
                    self.hwnd,
                    title_renderer,
                    text_color,
                );
            }
        }

        if self.show {
            return;
        }
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_SHOW);
            let _ = SetFocus(Some(self.hwnd));
        }
        self.show = true;
    }

    pub fn unpaint(&mut self) {
        drop(self.session.take());
        unsafe {
            let _ = ShowWindow(self.hwnd, SW_HIDE);
        }
        self.show = false;
    }

    pub fn find_clicked_app_index(&self, state: &SwitchAppsState) -> Option<usize> {
        let cursor_pos = unsafe {
            let mut pos = POINT::default();
            let _ = GetCursorPos(&mut pos);
            pos
        };

        let (monitor_rect, dpi) = get_monitor_rect_and_dpi();
        let dpi_scale = dpi as f64 / 96.0;
        let icon_size_max = (ICON_SIZE_BASE as f64 * dpi_scale) as i32;
        let border_size = (WINDOW_BORDER_SIZE_BASE as f64 * dpi_scale) as i32;
        let icon_border = (ICON_BORDER_SIZE_BASE as f64 * dpi_scale) as i32;
        let title_height = (TITLE_HEIGHT_BASE as f64 * dpi_scale) as i32;
        let title_gap = (TITLE_GAP_BASE as f64 * dpi_scale) as i32;
        let title_min_width = (TITLE_MIN_WIDTH_BASE as f64 * dpi_scale) as i32;
        let metrics = LayoutMetrics {
            icon_size_max,
            border_size,
            icon_border,
            title_height,
            title_gap,
            title_min_width,
        };

        let Coordinate {
            x,
            y,
            item_size,
            icons_x,
            ..
        } = Coordinate::new(state.windows.len() as i32, metrics, monitor_rect);

        let xpos = cursor_pos.x - x;
        let ypos = cursor_pos.y - y;

        let cy = border_size;
        for (i, _) in state.windows.iter().enumerate() {
            let cx = icons_x + item_size * (i as i32);
            if xpos >= cx && xpos < cx + item_size && ypos >= cy && ypos < cy + item_size {
                return Some(i);
            }
        }
        None
    }
}

impl Drop for GdiAAPainter {
    fn drop(&mut self) {
        drop(self.session.take());
        drop(self.title_renderer.take());
        unsafe {
            ReleaseDC(Some(self.hwnd), self.hdc_screen);
            GdiplusShutdown(self.token);
        }
    }
}

const fn theme_color(light_theme: bool) -> (u32, u32, u32) {
    match light_theme {
        true => (FG_LIGHT_COLOR, BG_LIGHT_COLOR, TEXT_LIGHT_COLOR),
        false => (FG_DARK_COLOR, BG_DARK_COLOR, TEXT_DARK_COLOR),
    }
}

unsafe fn draw_round_rect(
    graphic_ptr: *mut GpGraphics,
    brush_ptr: *mut GpBrush,
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
    corner_radius: f32,
) {
    unsafe {
        let mut path = GpPath::default();
        let mut path_ptr: *mut GpPath = &mut path;
        GdipCreatePath(FillModeAlternate, &mut path_ptr as _);
        GdipAddPathArc(
            path_ptr,
            left,
            top,
            corner_radius,
            corner_radius,
            180.0,
            90.0,
        );
        GdipAddPathArc(
            path_ptr,
            right - corner_radius,
            top,
            corner_radius,
            corner_radius,
            270.0,
            90.0,
        );
        GdipAddPathArc(
            path_ptr,
            right - corner_radius,
            bottom - corner_radius,
            corner_radius,
            corner_radius,
            0.0,
            90.0,
        );
        GdipAddPathArc(
            path_ptr,
            left,
            bottom - corner_radius,
            corner_radius,
            corner_radius,
            90.0,
            90.0,
        );
        GdipClosePathFigure(path_ptr);
        GdipFillPath(graphic_ptr, brush_ptr, path_ptr);
        GdipDeletePath(path_ptr);
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_icons(
    state: &SwitchAppsState,
    hdc_screen: HDC,
    icon_size: i32,
    icon_border: i32,
    width: i32,
    height: i32,
    bg_color: u32,
) -> HBITMAP {
    let scaled_width = width * SCALE_FACTOR;
    let scaled_height = height * SCALE_FACTOR;
    let scaled_border_size = icon_border * SCALE_FACTOR;
    let scaled_icon_inner_size = icon_size * SCALE_FACTOR;
    let scaled_icon_outer_size = scaled_icon_inner_size + scaled_border_size * 2;

    unsafe {
        let hdc_tmp = CreateCompatibleDC(Some(hdc_screen));
        let bitmap_tmp = CreateCompatibleBitmap(hdc_screen, width, height);
        let old_bitmap_tmp = SelectObject(hdc_tmp, bitmap_tmp.into());

        let hdc_scaled = CreateCompatibleDC(Some(hdc_screen));
        let bitmap_scaled = CreateCompatibleBitmap(hdc_screen, scaled_width, scaled_height);
        let old_bitmap_scaled = SelectObject(hdc_scaled, bitmap_scaled.into());

        let bg_brush = CreateSolidBrush(COLORREF(bg_color));

        let rect = RECT {
            left: 0,
            top: 0,
            right: scaled_width,
            bottom: scaled_height,
        };

        FillRect(hdc_scaled, &rect, bg_brush);

        for (i, window) in state.windows.iter().enumerate() {
            let cx = scaled_border_size + scaled_icon_outer_size * (i as i32);
            let _ = DrawIconEx(
                hdc_scaled,
                cx,
                scaled_border_size,
                window.icon,
                scaled_icon_inner_size,
                scaled_icon_inner_size,
                0,
                None,
                DI_NORMAL,
            );
        }

        SetStretchBltMode(hdc_tmp, HALFTONE);
        let _ = StretchBlt(
            hdc_tmp,
            0,
            0,
            width,
            height,
            Some(hdc_scaled),
            0,
            0,
            scaled_width,
            scaled_height,
            SRCCOPY,
        );

        let _ = DeleteObject(bg_brush.into());
        SelectObject(hdc_scaled, old_bitmap_scaled);
        let _ = DeleteObject(bitmap_scaled.into());
        let _ = DeleteDC(hdc_scaled);
        SelectObject(hdc_tmp, old_bitmap_tmp);
        let _ = DeleteDC(hdc_tmp);

        bitmap_tmp
    }
}

#[allow(clippy::too_many_arguments)]
fn draw_selected_icon(
    icon: windows::Win32::UI::WindowsAndMessaging::HICON,
    hdc_screen: HDC,
    icon_size: i32,
    icon_border: i32,
    item_size: i32,
    corner_radius: i32,
    fg_color: u32,
    bg_color: u32,
) -> HBITMAP {
    let scaled_size = item_size * SCALE_FACTOR;
    let scaled_icon_size = icon_size * SCALE_FACTOR;
    let scaled_border = icon_border * SCALE_FACTOR;
    let scaled_corner_radius = corner_radius * SCALE_FACTOR;

    unsafe {
        let hdc_target = CreateCompatibleDC(Some(hdc_screen));
        let bitmap_target = CreateCompatibleBitmap(hdc_screen, item_size, item_size);
        let old_target = SelectObject(hdc_target, bitmap_target.into());

        let hdc_scaled = CreateCompatibleDC(Some(hdc_screen));
        let bitmap_scaled = CreateCompatibleBitmap(hdc_screen, scaled_size, scaled_size);
        let old_scaled = SelectObject(hdc_scaled, bitmap_scaled.into());

        let bg_brush = CreateSolidBrush(COLORREF(bg_color));
        let fg_brush = CreateSolidBrush(COLORREF(fg_color));
        let rect = RECT {
            left: 0,
            top: 0,
            right: scaled_size,
            bottom: scaled_size,
        };
        FillRect(hdc_scaled, &rect, bg_brush);
        let region = CreateRoundRectRgn(
            0,
            0,
            scaled_size,
            scaled_size,
            scaled_corner_radius,
            scaled_corner_radius,
        );
        let _ = FillRgn(hdc_scaled, region, fg_brush);
        let _ = DeleteObject(region.into());
        let _ = DrawIconEx(
            hdc_scaled,
            scaled_border,
            scaled_border,
            icon,
            scaled_icon_size,
            scaled_icon_size,
            0,
            None,
            DI_NORMAL,
        );

        SetStretchBltMode(hdc_target, HALFTONE);
        let _ = StretchBlt(
            hdc_target,
            0,
            0,
            item_size,
            item_size,
            Some(hdc_scaled),
            0,
            0,
            scaled_size,
            scaled_size,
            SRCCOPY,
        );

        let _ = DeleteObject(bg_brush.into());
        let _ = DeleteObject(fg_brush.into());
        SelectObject(hdc_scaled, old_scaled);
        let _ = DeleteObject(bitmap_scaled.into());
        let _ = DeleteDC(hdc_scaled);
        SelectObject(hdc_target, old_target);
        let _ = DeleteDC(hdc_target);
        bitmap_target
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct Coordinate {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    icon_size: i32,
    item_size: i32,
    icons_x: i32,
    title_top: i32,
}

#[derive(Clone, Copy)]
struct LayoutMetrics {
    icon_size_max: i32,
    border_size: i32,
    icon_border: i32,
    title_height: i32,
    title_gap: i32,
    title_min_width: i32,
}

impl Coordinate {
    fn new(num_apps: i32, metrics: LayoutMetrics, monitor_rect: RECT) -> Self {
        let LayoutMetrics {
            icon_size_max,
            border_size,
            icon_border,
            title_height,
            title_gap,
            title_min_width,
        } = metrics;
        let monitor_width = monitor_rect.right - monitor_rect.left;
        let monitor_height = monitor_rect.bottom - monitor_rect.top;

        let icon_size = ((monitor_width - 2 * border_size) / num_apps - icon_border * 2)
            .clamp(1, icon_size_max.max(1));

        let item_size = icon_size + icon_border * 2;
        let icons_width = item_size * num_apps;
        let content_width = icons_width.max(title_min_width);
        let width = content_width + border_size * 2;
        let height = item_size + title_gap + title_height + border_size * 2;
        let x = monitor_rect.left + (monitor_width - width) / 2;
        let y = monitor_rect.top + (monitor_height - height) / 2;
        let icons_x = (width - icons_width) / 2;
        let title_top = border_size + item_size + title_gap;

        Self {
            x,
            y,
            width,
            height,
            icon_size,
            item_size,
            icons_x,
            title_top,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_keeps_valid_dimensions_with_many_windows() {
        let metrics = LayoutMetrics {
            icon_size_max: 64,
            border_size: 10,
            icon_border: 4,
            title_height: 26,
            title_gap: 4,
            title_min_width: 320,
        };
        let coordinate = Coordinate::new(
            500,
            metrics,
            RECT {
                left: 0,
                top: 0,
                right: 1920,
                bottom: 1080,
            },
        );
        assert!(coordinate.icon_size >= 1);
        assert!(coordinate.item_size > 0);
        assert!(coordinate.width > 0);
        assert!(coordinate.height > 0);
    }

    #[test]
    fn layout_centers_normal_window_sets() {
        let metrics = LayoutMetrics {
            icon_size_max: 64,
            border_size: 10,
            icon_border: 4,
            title_height: 26,
            title_gap: 4,
            title_min_width: 320,
        };
        let coordinate = Coordinate::new(
            5,
            metrics,
            RECT {
                left: 100,
                top: 50,
                right: 2020,
                bottom: 1130,
            },
        );
        assert_eq!(coordinate.icon_size, 64);
        assert_eq!(coordinate.x, 100 + (1920 - coordinate.width) / 2);
        assert_eq!(coordinate.y, 50 + (1080 - coordinate.height) / 2);
    }

    #[test]
    fn paint_sessions_do_not_leak_gdi_objects() {
        use windows::Win32::{
            System::Threading::{GetCurrentProcess, GetGuiResources, GR_GDIOBJECTS},
            UI::WindowsAndMessaging::{CopyIcon, DestroyIcon, LoadIconW, IDI_APPLICATION},
        };

        unsafe {
            let mut token = 0usize;
            let startup_input = GdiplusStartupInput {
                GdiplusVersion: 1,
                ..Default::default()
            };
            assert_eq!(
                GdiplusStartup(&mut token, &startup_input, std::ptr::null_mut()).0,
                0
            );
            let hdc_screen = GetDC(None);
            let shared_icon = LoadIconW(None, IDI_APPLICATION).unwrap();
            let icon = CopyIcon(shared_icon).unwrap();
            let state = SwitchAppsState {
                windows: vec![crate::app::WindowEntry {
                    icon,
                    hwnd: HWND::default(),
                    title: "Test".into(),
                    icon_key: "test".into(),
                }],
                index: 0,
            };
            let spec = PaintSpec {
                coordinate: Coordinate {
                    x: 0,
                    y: 0,
                    width: 340,
                    height: 122,
                    icon_size: 64,
                    item_size: 72,
                    icons_x: 134,
                    title_top: 86,
                },
                border_size: 10,
                icon_border: 4,
                title_height: 26,
                corner_radius: 18,
                fg_color: FG_DARK_COLOR,
                bg_color: BG_DARK_COLOR,
            };

            drop(PaintSession::new(&state, hdc_screen, spec));
            let process = GetCurrentProcess();
            let before = GetGuiResources(process, GR_GDIOBJECTS);
            for _ in 0..500 {
                drop(PaintSession::new(&state, hdc_screen, spec));
            }
            let after = GetGuiResources(process, GR_GDIOBJECTS);

            let _ = DestroyIcon(icon);
            ReleaseDC(None, hdc_screen);
            GdiplusShutdown(token);
            assert!(
                after <= before + 2,
                "GDI objects grew from {before} to {after}"
            );
        }
    }
}
