//! OS 標準の UI フォントを、表示される各ウィンドウの既定フォントにする
//! (オプトイン: アプリが [`apply_os_default()`](ui_font::apply_os_default)
//! を呼んだときだけ働く。呼ばなければ最終バイナリに含まれない)。
//!
//! OS ごとの挙動、仕組み、.slint 側の宣言要件は
//! [`apply_os_default()`](ui_font::apply_os_default) の doc を参照。

use std::sync::atomic::{AtomicBool, Ordering::Relaxed};

use i_slint_core::item_tree::ItemRc;
use i_slint_core::items::WindowItem;
use slint::private_unstable_api::re_exports as sp;

static INSTALLED: AtomicBool = AtomicBool::new(false);
/// デスクトップ環境の UI フォント ファミリ (プロセスで 1 回だけ読む。Linux のみ)。
#[cfg(target_os = "linux")]
static CACHED_FAMILY: std::sync::OnceLock<Option<String>> = std::sync::OnceLock::new();

/// 表示される各ウィンドウの既定フォントを、OS 標準の UI フォントへ揃える
/// (オプトイン。呼ばないときはこの機能は最終バイナリに含まれない)。
///
/// - **Windows**: Yu Gothic UI。slint はファミリ指定が無いと Segoe UI へ落
///   ちるが、Segoe UI は和文を持たないため和文だけ別書体へフォールバックし、
///   欧文と和文で書体が混在する。Yu Gothic UI は和欧両対応のため混在が起き
///   ない (Windows 10/11 の日本語 UI の標準書体でもある)。
/// - **Linux**: デスクトップ環境の「インターフェース用フォント」(GTK の
///   `gtk-font-name`)。slint はファミリ指定が無いとき fontconfig の
///   `sans-serif` 汎用別名へ落ちるが、これはデスクトップ環境の UI フォント
///   とは別物で、周囲の GTK アプリと書体がズレるため。ファミリ名はハード
///   コードしない (デスクトップ環境の設定に追従する)。
/// - **macOS**: 何もしない (システム既定の解決で和欧も自然に揃う)。
///
/// 仕組み: slint core の window shown フック (show() の中、set_visible の直
/// 後 = 最初の描画より前) を 1 回仕込み、表示される各ウィンドウの
/// `WindowItem.default_font_family` へ差し込む。メイン ウィンドウ以外のす
/// べてのウィンドウ (設定ダイアログ / バージョン ダイアログ等) もカバーす
/// る。
///
/// 呼ぶタイミング: 最初のコンポーネント生成後 (プラットフォーム初期化後)。
/// `platform::setup_window` の直後が自然。プロセスで 1 回でよい (2 回呼ん
/// でも 2 回目は何もしない)。
///
/// アプリが自前のファミリを明示したい場合は、`default-font-family` の参照先
/// プロパティへ空でない値を入れる (ライブラリは空のときだけ OS 標準へ揃え
/// る)。その場合も動的バインディングの宣言は必要 (下記)。
///
/// 注: アプリの .slint が `default-font-family` を動的バインディングで宣言
/// している場合にのみ働く (未宣言やリテラル固定だとコンパイラが定数化する
/// ため書き込めず、panic 防止のガードで何もしない)。timekeeper.slint の記
/// 述を参照:
/// `in property <string> ui-font-family: "";` + `default-font-family:
/// root.ui-font-family;`
pub fn apply_os_default() {
	// set_window_shown_hook はプラットフォーム未初期化のとき Err を返すが、
	// コンポーネント生成後の時点では初期化済み。念のため Err は握りつぶし
	// て既定動作に任せる。
	if INSTALLED.swap(true, Relaxed) {
		return;
	}
	let _ = i_slint_core::context::set_window_shown_hook(Some(Box::new(|adapter| {
		if let Some(family) = default_ui_font_family() {
			apply_to_window(adapter, &family);
		}
	})));
}

/// OS 標準の UI フォント ファミリ。Windows は Yu Gothic UI (和欧混植の防止。
/// モジュール doc 参照)、Linux はデスクトップ環境の設定 (gtk-font-name)、
/// macOS などそれ以外は None (システム既定のまま)。
fn default_ui_font_family() -> Option<&'static str> {
	#[cfg(target_os = "windows")]
	{
		Some("Yu Gothic UI")
	}
	#[cfg(target_os = "linux")]
	{
		de_ui_font_family()
	}
	#[cfg(not(any(windows, target_os = "linux")))]
	{
		None
	}
}

/// デスクトップ環境の UI フォント ファミリ。GTK の `gtk-font-name` プロパティ
/// (例: "Ubuntu 11" / "Adwaita Sans 11") を Pango のフォント記述としてパース
/// し、ファミリ名だけを取り出す (スタイル / サイズはアプリの既定に任せる)。
/// GTK を初期化できない環境 (設定が無い / GNOME 以外で GTK の設定が無い) で
/// は None を返し、何もしない (slint の既定フォントのまま)。
#[cfg(target_os = "linux")]
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
/// もせず slint の既定フォントのままにする。機能を有効にするにはアプリの
/// .slint で `in property <string>` への動的バインディングとして
/// `default-font-family` を宣言すること (timekeeper.slint の記述を参照)。
///
/// また、アプリが明示的にファミリを設定している (現在値が空でない) 場合は
/// それを尊重する。動的バインディングの宣言は「実行時へ差し込めるようにす
/// る」ためでもあるが、アプリが自前のファミリを入れたときにライブラリが上
/// 書きしないための目印でもある (空 = アプリは指定していない = OS 標準へ揃
/// える)。
fn apply_to_window(adapter: &sp::Rc<dyn sp::WindowAdapter>, family: &str) {
	let window = adapter.window();
	let Some(root) = sp::WindowInner::from_pub(window).try_component() else { return };
	let root_item = ItemRc::new_root(root);
	let Some(window_item) = root_item.downcast::<WindowItem>() else { return };
	let window_item = window_item.as_pin_ref();
	if window_item.default_font_family.is_constant() {
		return;
	}
	// Pin<&WindowItem> からフィールドへの Pin プロジェクション (標準的な形)。
	// WindowItem はコンポーネント ツリー内で pinned であり、フィールドのアド
	// レスは安定しているため投影は安全 (取り出すのは参照だけ)。
	let family_prop = unsafe {
		window_item.as_ref().map_unchecked(|w| &w.default_font_family)
	};
	// アプリが明示的にファミリを設定している (空でない) 場合は尊重する。
	// 動的バインディングの宣言は「実行時へ差し込めるようにする」ためでもあ
	// るが、アプリが自前のファミリを入れたときにライブラリが上書きしないた
	// めの目印でもある (空 = アプリは指定していない = OS 標準へ揃える)。
	if !family_prop.get().trim().is_empty() {
		return;
	}
	window_item.default_font_family.set(family.into());
}
