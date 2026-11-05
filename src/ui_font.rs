//! Linux の UI フォント (デスクトップ環境の設定) を各ウィンドウの既定
//! フォントにする (ライブラリ内部処理)。
//!
//! slint はフォント ファミリの指定が無いとき fontconfig の `sans-serif` 汎用
//! 別名へ落ちる (parley/fontique のフォールバック連鎖の先頭が SansSerif の
//! ため)。これはディストリビューションのフォント設定であって、デスクトップ
//! 環境の「インターフェース用フォント」とは別物 (Ubuntu では sans-serif が
//! DejaVu Sans に解決される一方、GNOME アプリは `org.gnome.desktop.interface
//! font-name` の書体を使う)。そのため既定のままでは周囲の GTK アプリと書体
//! がズレて見える。
//!
//! そこで GTK の `gtk-font-name` (デスクトップ環境の UI フォント設定そのも
//! の。GNOME はここへ gsettings の値を流す) を実行時に読み、Pango のフォン
//! ト記述としてパースしたファミリ名を、表示される各ウィンドウの
//! `WindowItem.default_font_family` (slint core の公開型) へ書き込む。ファミ
//! リ名はハードコードしない (デスクトップ環境の設定に追従する)。
//!
//! 書き込みのタイミングは slint core の window shown フック (show() の中、
//! set_visible の直後 = 最初の描画より前) で、メイン ウィンドウ以外のすべ
//! てのウィンドウ (設定ダイアログ / バージョン ダイアログ等) もカバーする。
//! アプリの .slint が `default-font-family` を動的バインディングで宣言して
//! いる場合は、表示時点では初期化が終わっているのでこの値が優先される
//! (Linux では本機能が既定)。宣言が無い / リテラル固定のウィンドウではコ
//! ンパイラが定数化するため書き込めず (panic防止のガードで何もしない)、
//! fontconfig の sans-serif のままになる。
//!
//! 注: ツールチップなどのポップアップ ウィンドウは show() を通らないため対
//! 象外 (fontconfig の sans-serif のまま)。ドロップダウン メニューは
//! in-window オーバーレイなのでウィンドウと同じフォントになる。

use std::sync::atomic::{AtomicBool, Ordering::Relaxed};
use std::sync::OnceLock;

use i_slint_core::item_tree::ItemRc;
use i_slint_core::items::WindowItem;
use slint::private_unstable_api::re_exports as sp;

static INSTALLED: AtomicBool = AtomicBool::new(false);
/// デスクトップ環境の UI フォント ファミリ (プロセスで 1 回だけ読む)。
static CACHED_FAMILY: OnceLock<Option<String>> = OnceLock::new();

/// shown フックを仕込む (setup_window から呼ぶ。プロセスで 1 回)。
pub(crate) fn install_window_font_hook() {
	if INSTALLED.swap(true, Relaxed) {
		return;
	}
	// set_window_shown_hook はプラットフォーム未初期化のとき Err を返すが、
	// setup_window の時点では初期化済み (コンポーネント生成で初期化される)。
	// 念のため Err は握りつぶして既定動作 (sans-serif) に任せる。
	let _ = i_slint_core::context::set_window_shown_hook(Some(Box::new(|adapter| {
		if let Some(family) = de_ui_font_family() {
			apply_to_window(adapter, &family);
		}
	})));
}

/// デスクトップ環境の UI フォント ファミリ。GTK の `gtk-font-name` プロパティ
/// (例: "Ubuntu 11" / "Adwaita Sans 11") を Pango のフォント記述としてパース
/// し、ファミリ名だけを取り出す (スタイル / サイズはアプリの既定に任せる)。
/// GTK を初期化できない環境 (設定が無い / GNOME 以外で GTK の設定が無い) で
/// は None を返し、何もしない (fontconfig の sans-serif のまま)。
fn de_ui_font_family() -> Option<&'static str> {
	CACHED_FAMILY
		.get_or_init(|| {
			if !crate::gtk::gtk_ready() {
				return None;
			}
			use gtk::prelude::*;
			let settings = gtk::Settings::default()?;
			let font_name = settings.gtk_font_name()?;
			let desc = gtk::pango::FontDescription::from_string(&font_name);
			desc.family().map(|s| s.to_string())
		})
		.as_deref()
}

/// ウィンドウのルート項目 (WindowItem — slint core の公開型) へファミリを
/// 書き込む。Property::set はバインディングを置き換えるので、以後この値が
/// 使われる。
///
/// ただし `default-font-family` が宣言されていない (またはリテラル固定の)
/// ウィンドウでは、コンパイラがこのプロパティを読み取り側も含めて定数化す
/// るため、実行時の書き込みは不可能 (set すると panic する)。その場合は何
/// もせず fontconfig の sans-serif のままにする。機能を有効にするにはアプ
/// リの .slint で `in property <string>` への動的バインディングとして
/// `default-font-family` を宣言すること (timekeeper.slint の記述を参照)。
fn apply_to_window(adapter: &sp::Rc<dyn sp::WindowAdapter>, family: &str) {
	let window = adapter.window();
	let Some(root) = sp::WindowInner::from_pub(window).try_component() else { return };
	let root_item = ItemRc::new_root(root);
	let Some(window_item) = root_item.downcast::<WindowItem>() else { return };
	let window_item = window_item.as_pin_ref();
	if window_item.default_font_family.is_constant() {
		return;
	}
	window_item.default_font_family.set(family.into());
}
