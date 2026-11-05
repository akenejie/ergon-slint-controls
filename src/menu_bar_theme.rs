//! メニューバー / メニューのテーマ馴染ませ (ライブラリ内部処理)。
//!
//! ui/menu.slint の `MenuBarTheme` グローバルは、アプリが
//! `@ergon_slint_controls` で部品を参照すると from_library として共有される
//! (型は本ライブラリの生成コード、実体はアプリのルート コンポーネントごと)。
//! 実体は lib.rs の `ergon_slint_controlsSharedGlobals` ラッパーが生成時に
//! [`register`] へ渡す。Linux では [`setup_window`][crate::platform::setup_window]
//! が GTK テーマから取得した色を全実体へ流し込む。アプリ側に必要なコードは
//! 無い。Windows / macOS は流し込みを行わないので既定値のまま (見た目不変)。
//!
//! 実体はスレッドをまたげない (Property / Rc) ので、登録と流し込みは
//! スレッド ローカルで管理する。slint のコンポーネント生成と setup_window
//! は同じ (メイン) スレッドで行う前提。

use std::cell::RefCell;
use std::pin::Pin;
use std::rc::Rc;

use slint::private_unstable_api::re_exports as sp;

/// 登録する実体 (ui/menu.slint の MenuBarTheme グローバルの生成型)。
type ThemeGlobal = Pin<Rc<crate::generated::InnerMenuBarTheme>>;
/// 実体が属するルート コンポーネント (弱参照。消滅後に掃除する)。
type RootWeak = sp::VWeak<sp::ItemTreeVTable>;

/// メニューバー / メニューのテーマ色。
#[cfg(target_os = "linux")]
#[derive(Clone, Copy, Debug)]
pub(crate) struct MenuBarColors {
	/// メニューバー (バー本体) の背景。
	pub bar_background: slint::Color,
	/// バー下端の下線。
	pub bar_underline: slint::Color,
	/// バー項目の文字色。
	pub bar_foreground: slint::Color,
	/// ドロップダウン メニューの背景。
	pub menu_background: slint::Color,
	/// ドロップダウン項目の文字色。
	pub menu_foreground: slint::Color,
	/// ドロップダウン項目のハイライト (hover / キーボード選択) 背景。
	pub menu_hover_background: slint::Color,
	/// 暗色テーマか (hover などの半透明オーバーレイの白黒反転用)。
	pub dark: bool,
}

/// 既定 (Windows 10 標準風の固定値)。ui/menu.slint の既定値と同じ。
#[cfg(target_os = "linux")]
fn default_colors() -> MenuBarColors {
	MenuBarColors {
		bar_background: slint::Color::from_argb_encoded(0xFFFF_FFFF),
		bar_underline: slint::Color::from_argb_encoded(0xFFE5_E5E5),
		bar_foreground: slint::Color::from_argb_encoded(0xE600_0000),
		menu_background: slint::Color::from_argb_encoded(0xFFFA_FAFA),
		menu_foreground: slint::Color::from_argb_encoded(0xE600_0000),
		menu_hover_background: slint::Color::from_argb_encoded(0x0A00_0000),
		dark: false,
	}
}

/// 取得済みの色 (プロセスで 1 回だけ計算する。テーマは実行中に変わらない
/// 前提。Chromium もテーマ読み込み時にしか再取得しない)。
#[cfg(target_os = "linux")]
static CACHED: std::sync::OnceLock<MenuBarColors> = std::sync::OnceLock::new();

thread_local! {
	/// 共有された MenuBarTheme の実体 (登録順)。ルートが消えたら掃除。
	static REGISTERED: RefCell<Vec<(RootWeak, ThemeGlobal)>> = const { RefCell::new(Vec::new()) };
}

/// アプリの生成コードがルート コンポーネントを作ったときに
/// `ergon_slint_controlsSharedGlobals` ラッパー (lib.rs) から呼ばれる。
/// 実体を登録するだけ (色の適用は [`apply_computed`] と
/// [`reapply_after_init`] が担う)。
pub(crate) fn register(root: RootWeak, theme: ThemeGlobal) {
	REGISTERED.with(|registered| {
		let mut list = registered.borrow_mut();
		list.retain(|(root, _)| root.upgrade().is_some());
		list.push((root, theme));
	});
}

/// GTK テーマから色を取得 (プロセスで初回のみ。失敗時は既定値) し、登録済み
/// の全実体へ流し込む。setup_window が呼ぶ (メイン スレッド。GTK の初期化を
/// 伴う)。
#[cfg(target_os = "linux")]
pub(crate) fn apply_computed() {
	let colors = CACHED.get_or_init(query_gtk_menu_bar_theme);
	REGISTERED.with(|registered| {
		let mut list = registered.borrow_mut();
		list.retain(|(root, _)| root.upgrade().is_some());
		for (_, theme) in list.iter() {
			apply_one(theme, colors);
		}
	});
}

/// ルートの生成直後 (生成コードの init_globals が既定色を書き込んだ後) に
/// ラッパーから呼ばれる。既に色を取得済みならその実体へ反映する (後から
/// 生成されたルート向け。最初のルートはまだ色が無いので既定値のまま、
/// setup_window で流し込まれる)。
pub(crate) fn reapply_after_init(theme: &ThemeGlobal) {
	#[cfg(target_os = "linux")]
	if let Some(colors) = CACHED.get() {
		apply_one(theme, colors);
	}
	#[cfg(not(target_os = "linux"))]
	let _ = theme;
}

/// 実体へ色を書き込む。ui/menu.slint の既定値は定数なのでバインディングは
/// 無く、以後の Property::set がそのまま効く。
#[cfg(target_os = "linux")]
fn apply_one(theme: &ThemeGlobal, colors: &MenuBarColors) {
	theme.bar_background.set(colors.bar_background);
	theme.bar_underline.set(colors.bar_underline);
	theme.bar_foreground.set(colors.bar_foreground);
	theme.menu_background.set(colors.menu_background);
	theme.menu_foreground.set(colors.menu_foreground);
	theme.menu_hover_background.set(colors.menu_hover_background);
	theme.dark.set(colors.dark);
}

#[cfg(target_os = "linux")]
fn rgba_to_color(c: gtk::gdk::RGBA) -> slint::Color {
	let to8 = |v: f64| -> u32 { (v.clamp(0.0, 1.0) * 255.0).round() as u32 };
	slint::Color::from_argb_encoded(
		(to8(c.alpha()) << 24) | (to8(c.red()) << 16) | (to8(c.green()) << 8) | to8(c.blue()),
	)
}

#[cfg(target_os = "linux")]
fn color_luma(c: slint::Color) -> f64 {
	let r = f64::from(c.red()) / 255.0;
	let g = f64::from(c.green()) / 255.0;
	let b = f64::from(c.blue()) / 255.0;
	// Chromium color_utils::IsDark と同程度の相対輝度近似。
	0.299 * r + 0.587 * g + 0.114 * b
}

#[cfg(target_os = "linux")]
fn mix_color(a: slint::Color, b: slint::Color, t: f64) -> slint::Color {
	let m = |x: u8, y: u8| -> u32 { (f64::from(x) * (1.0 - t) + f64::from(y) * t).round() as u32 };
	slint::Color::from_argb_encoded(
		(u32::from(a.alpha()) << 24)
			| (m(a.red(), b.red()) << 16)
			| (m(a.green(), b.green()) << 8)
			| m(a.blue(), b.blue()),
	)
}

#[cfg(target_os = "linux")]
/// 背景の抽出 (Chromium ui/gtk/gtk_util.cc の GetBgColorFromStyleContext)。
/// 背景はグラデーション / 背景画像のことがあるため background-color は使わ
/// ず、境界を消した上で 24x24 に描画して平均色を取る。親チェーンも描画する
/// (窓の背景が下に敷かれる)。
fn gtk_bg_color(ctx: &gtk::StyleContext) -> Option<slint::Color> {
	use gtk::prelude::*;
	// 境界を消す (Chromium の strip_borders CSS プロバイダと同じ)。
	let provider = gtk::CssProvider::new();
	provider
		.load_from_data(b"* { border-radius: 0px; border-style: none; box-shadow: none; }")
		.ok()?;
	{
		let mut chain = Some(ctx.clone());
		while let Some(c) = chain {
			c.add_provider(&provider, gtk::STYLE_PROVIDER_PRIORITY_APPLICATION as u32);
			chain = c.parent();
		}
	}
	let mut surface = gtk::cairo::ImageSurface::create(gtk::cairo::Format::ARgb32, 24, 24).ok()?;
	let cr = gtk::cairo::Context::new(&surface).ok()?;
	// 親から子へ描画 (Chromium の RenderBackground)。
	fn render(ctx: Option<&gtk::StyleContext>, cr: &gtk::cairo::Context) {
		if let Some(ctx) = ctx {
			render(ctx.parent().as_ref(), cr);
			// gtk3 クレートではメソッドでなく自由関数。
			gtk::render_background(ctx, cr, 0.0, 0.0, 24.0, 24.0);
		}
	}
	render(Some(ctx), &cr);
	// data() は排他参照を要求する (NonExclusive) ので、surface を参照している
	// Context を先に捨てる。
	drop(cr);
	surface.flush();
	let data = surface.data().ok()?;
	let (mut r, mut g, mut b, mut n) = (0u64, 0u64, 0u64, 0u64);
	for px in data.chunks_exact(4) {
		// cairo ARGB32 はホスト エンディアンの premultiplied ARGB (x86 は BGRA)。
		let (b8, g8, r8, a8) =
			(u32::from(px[0]), u32::from(px[1]), u32::from(px[2]), u32::from(px[3]));
		if a8 == 0 {
			continue;
		}
		// premultiplied を戻して平均に足す。
		r += u64::from(r8 * 255 / a8.max(1));
		g += u64::from(g8 * 255 / a8.max(1));
		b += u64::from(b8 * 255 / a8.max(1));
		n += 1;
	}
	if n == 0 {
		return None;
	}
	Some(slint::Color::from_argb_encoded(
		0xFF00_0000
			| ((r / n) as u32) << 16
			| ((g / n) as u32) << 8
			| (b / n) as u32,
	))
}

#[cfg(target_os = "linux")]
/// 文字色の抽出 (Chromium の ComputeFgColor)。アルファ付きなら背景にブレンド。
fn gtk_fg_color(ctx: &gtk::StyleContext, bg: Option<slint::Color>) -> Option<slint::Color> {
	use gtk::prelude::*;
	let rgba = ctx.color(gtk::StateFlags::NORMAL);
	let a = rgba.alpha();
	if a >= 1.0 {
		return Some(rgba_to_color(rgba));
	}
	let bg = bg?;
	let m = |f: f64, b: u8| -> u8 { (f * a * 255.0 + f64::from(b) * (1.0 - a)).round() as u8 };
	Some(slint::Color::from_argb_encoded(
		0xFF00_0000
			| (u32::from(m(rgba.red(), bg.red())) << 16)
			| (u32::from(m(rgba.green(), bg.green())) << 8)
			| u32::from(m(rgba.blue(), bg.blue())),
	))
}

#[cfg(target_os = "linux")]
fn query_gtk_menu_bar_theme() -> MenuBarColors {
	use gtk::prelude::*;
	if !crate::gtk::gtk_ready() {
		return default_colors();
	}
	// ダミー窓 (表示しない。Chromium ui/gtk/gtk_util.cc の CreateDummyWindow
	// と同じ。realize だけでマップしないので WM には現れない)。
	let win = gtk::Window::new(gtk::WindowType::Toplevel);
	// フレーム (タイトルバー / メニューバー) 相当。
	// Chromium: GetBgColor("headerbar.header-bar.titlebar")。
	let header = gtk::HeaderBar::new();
	win.set_titlebar(Some(&header));
	// メニュー (Chromium: GetBgColor("menu") / GetFgColor("menu menuitem label")
	// / GetBgColor("menu menuitem:hover"))。実ウィジェットを親チェーンに組む
	// ことでテーマの CSS 継承が効く。
	let menubar = gtk::MenuBar::new();
	let item = gtk::MenuItem::with_label("Menu");
	let menu = gtk::Menu::new();
	let sub = gtk::MenuItem::with_label("Item");
	menu.append(&sub);
	item.set_submenu(Some(&menu));
	menubar.append(&item);
	win.add(&menubar);
	win.realize();

	let header_ctx = header.style_context();
	// バー本体は GtkMenuBar の背景を優先 (描くのはメニューバーであって
	// タイトルバーではない)。テーマによっては menubar が透明のことがあるの
	// で、その場合はヘッダーバー (Chromium の frame 色と同じ) へフォール
	// バックする。
	let menubar_ctx = menubar.style_context();
	let Some(bar_background) = gtk_bg_color(&menubar_ctx).or_else(|| gtk_bg_color(&header_ctx))
	else {
		return default_colors();
	};
	// バー項目の文字色はバー項目のラベル (GtkLabel) から取る (バー自体は
	// 文字色を持たない)。ヘッダーバーの文字色はテーマのアクセントが乗る
	// ことがあるため使わない。
	let bar_label_ctx = item
		.child()
		.and_then(|c| c.downcast::<gtk::Label>().ok())
		.map(|label| label.style_context())
		.unwrap_or_else(|| menubar_ctx.clone());
	let bar_foreground = gtk_fg_color(&bar_label_ctx, Some(bar_background))
		.or_else(|| gtk_fg_color(&header_ctx, Some(bar_background)))
		.unwrap_or(slint::Color::from_argb_encoded(0xE600_0000));
	// 下線は背景と文字の中間 (テーマ依存の境界色クエリをしない簡易派生)。
	let bar_underline = mix_color(bar_background, bar_foreground, 0.12);

	let menu_ctx = menu.style_context();
	let Some(menu_background) = gtk_bg_color(&menu_ctx) else {
		return default_colors();
	};
	// 項目の文字色はラベル (GtkLabel) のスタイルから取る (MenuItem 自体は
	// 文字色を持たない)。
	let sub_ctx = sub
		.child()
		.and_then(|c| c.downcast::<gtk::Label>().ok())
		.map(|label| label.style_context())
		.unwrap_or_else(|| sub.style_context());
	let menu_foreground = gtk_fg_color(&sub_ctx, Some(menu_background))
		.unwrap_or(slint::Color::from_argb_encoded(0xE600_0000));
	// ハイライト (hover) 背景: Chromium の GetBgColor("menu menuitem:hover")。
	let item_ctx = item.style_context();
	item_ctx.set_state(gtk::StateFlags::PRELIGHT);
	let menu_hover_background = gtk_bg_color(&item_ctx).unwrap_or_else(|| {
		// フォールバック: 背景と文字の中間の半透明。
		mix_color(menu_background, menu_foreground, 0.08)
	});

	let win_bg = gtk_bg_color(&win.style_context()).unwrap_or(bar_background);
	let dark = color_luma(win_bg) < 0.5;

	let colors = MenuBarColors {
		bar_background,
		bar_underline,
		bar_foreground,
		menu_background,
		menu_foreground,
		menu_hover_background,
		dark,
	};
	colors
}
