//! モーダル ダイアログ制御ライブラリ。
//!
//! アプリは「ダイアログの中身」と「開く/閉じる指示」だけを書き、OS 間の差分
//! (子ウィンドウの生成属性・タイトルバー整え・中央配置・Esc/Enter の読み替え・
//! 親ウィンドウの入力遮断・破棄) はこのクレートが担う。
//!
//! # 設計思想
//!
//! PC のウィンドウはウィンドウ マネージャ (WM) の管理下にあり、利用者は WM
//! を通じてウィンドウを操作する。アプリが勝手な制御 (常に最面等) を挟むと
//! 利用者の操作を阻害する。そこでこのクレートは 2 つの穏当な方式だけを提供
//! する:
//!
//! - [`ModalMode::InWindowOverlay`]: ダイアログをアプリ自身のウィンドウ内に
//!   完結させ、WM への干渉はゼロ。スマホや車載 (IVI) など、OS の哲学として
//!   多数のウィンドウを出さないプラットフォームにも適する。
//! - [`ModalMode::ChildWindow`]: WM の標準ヒント (所有ウィンドウ / transient /
//!   modal / dialog 型) を依頼するだけで、always-on-top などの強制はしない。
//!
//! # 表示方式の切り替え (アプリ側のフラグ)
//!
//! アプリの Cargo.toml で feature を選ぶだけで、OS 毎にバイナリへ焼かれる
//! 制御が変わる:
//!
//! ```toml
//! # 例: win/linux/macos 共にウィンドウ内オーバーレイにする
//! modal-window = { git = "...", tag = "...", features = ["force-in-window-overlay"] }
//! ```
//!
//! - `force-in-window-overlay` = 全 OS で [`ModalMode::InWindowOverlay`]
//! - `force-child-window` = 全 OS で [`ModalMode::ChildWindow`]
//! - どちらも無指定 = [`ModalMode::default_for_os`] (Windows = 子ウィンドウ、
//!   それ以外 = ウィンドウ内ポップアップ)
//!
//! [`ModalMode`] の値は [`MODAL_MODE`] 定数として公開されるので、アプリ側の
//! 分岐は `if modal_window::MODAL_MODE.is_in_window_overlay() { ... }` と
//! 書ける。
//!
//! [`MODAL_MODE`] は `const` なので、選ばれなかった側の実装は release ビルド
//! (LTO) で dead-code elimination により除去される。
//!
//! # 子ダイアログのスタイル (アイコン無しのダイアログ枠)
//!
//! モーダル子ダイアログの慣習的なスタイルは「アイコン無しのダイアログ枠」
//! (Windows: `WS_EX_DLGMODALFRAME`)。本クレートは
//! [`backend::prepare_backend`] の生成フックと、生成後の
//! [`child::enforce_modal_styles`] (Win32 で冪等に整える) の 2 段構えで
//! 適用するため、**fork 無しでも子ダイアログ モードは動く**。
//!
//! 追加で次の 2 点を求める場合だけ、任意の winit パッチ フォーク
//! (https://github.com/akenejie/slint-modal-winit) を feature
//! `winit-modal-hints` と併せて有効にする:
//!
//! - 生成時点から `WS_EX_DLGMODALFRAME` を与え、初回描画のちらつきを消す
//! - X11 で `WM_TRANSIENT_FOR` / `_NET_WM_STATE_MODAL` を付ける
//!   (無効時は `_NET_WM_WINDOW_TYPE_DIALOG` + 親入力の遮断で代替する)
//!
//! Cargo の `[patch.crates-io]` は**ビルドのルート (アプリ) の manifest で
//! しか有効にならず、ライブラリから消費側へ伝播しない**。したがってこの
//! feature を使うアプリが、自分の Cargo.toml に次の 1 行を置く:
//!
//! ```toml
//! [patch.crates-io]
//! winit = { git = "https://github.com/akenejie/slint-modal-winit", tag = "v0.30.13-modal.1" }
//! ```
//!
//! 本クレート自身は fork を参照しない (既定ビルドは crates.io の winit で
//! 完結する)。フォークは upstream winit v0.30.13 を基準にしたコミット列なので、
//! 差分は `git diff v0.30.13..main` で見え、上流追従は merge で行える。
//!
//! # 起動時の準備 (子ウィンドウ モードのみ意味を持つ)
//!
//! [`backend::prepare_backend`] を `main` 冒頭で呼ぶと、以降
//! [`backend::mark_next_window_as_child_dialog`] の直後に生成される Slint
//! ウィンドウが「親のモーダル子ダイアログ」用の OS 属性 (Windows: ダイアログ枠
//! + 所有ウィンドウ / X11: `_NET_WM_WINDOW_TYPE_DIALOG`) 付きで作られる。
//! オーバーレイ モードでは子ウィンドウを生成しないため、この準備は何もしない
//! (既定の winit バックエンドに任せる)。

pub mod backend;
pub mod child;

pub use backend::{mark_next_window_as_child_dialog, prepare_backend};
#[cfg(windows)]
pub use backend::{note_owner_hwnd, note_owner_window};
pub use child::{
	center_modal_dialog, close_child_modal, dismiss_modal_dialog, enforce_modal_styles,
	open_child_modal, ModalSession, ModalSlot, WeakModalSlot,
};

// --- 表示方式 (feature 1 箇所で OS 毎の挙動が変わる) ---

/// モーダルの表示方式。
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ModalMode {
	/// 別 OS ウィンドウの子ダイアログ。デスクトップでウィンドウ マネージャが
	/// 子ダイアログ (transient / dialog 型) を管理できる環境向け。
	ChildWindow,
	/// メインウィンドウ内の暗転オーバーレイ + パネル表示。スマホや車載 (IVI)
	/// など、OS の哲学として多数のウィンドウを出さないプラットフォーム向け。
	InWindowOverlay,
}

impl ModalMode {
	/// OS ごとの既定。
	///
	/// - Windows: デスクトップで子ダイアログの管理 (所有ウィンドウ・ダイアログ
	///   枠) が確実なため子ウィンドウ。
	/// - それ以外 (Linux / macOS / Android / iOS / 車載・組込み): ウィンドウ
	///   マネージャの対応が揃っていない、または OS の哲学として多数のウィンドウ
	///   を出さないため、単一ウィンドウ内のポップアップ。
	pub const fn default_for_os() -> Self {
		#[cfg(target_os = "windows")]
		{
			Self::ChildWindow
		}
		#[cfg(not(target_os = "windows"))]
		{
			Self::InWindowOverlay
		}
	}

	/// ウィンドウ内オーバーレイ方式か。`const` なので分岐は定数畳み込みされ、
	/// release ビルドでは選ばれない側の実装が残らない。
	pub const fn is_in_window_overlay(self) -> bool {
		matches!(self, Self::InWindowOverlay)
	}

	/// 子ウィンドウ方式か。[`is_in_window_overlay`](Self::is_in_window_overlay)
	/// の否定に相当。
	pub const fn is_child_window(self) -> bool {
		matches!(self, Self::ChildWindow)
	}
}

// force-* feature の二重指定は設定ミスなので弾く。
#[cfg(all(feature = "force-in-window-overlay", feature = "force-child-window"))]
compile_error!(
	"force-in-window-overlay と force-child-window は同時に指定できません"
);

/// このバイナリで使うモーダルの表示方式。アプリ側の Cargo.toml で feature を
/// 選ぶとここに焼かれる:
///
/// - `force-in-window-overlay` = 全 OS で [`ModalMode::InWindowOverlay`]
/// - `force-child-window` = 全 OS で [`ModalMode::ChildWindow`]
/// - 無指定 = [`ModalMode::default_for_os`]
pub const MODAL_MODE: ModalMode = if cfg!(feature = "force-in-window-overlay") {
	ModalMode::InWindowOverlay
} else if cfg!(feature = "force-child-window") {
	ModalMode::ChildWindow
} else {
	ModalMode::default_for_os()
};

/// 子ウィンドウ (別 OS ウィンドウ) モーダルのダイアログ側抽象。
///
/// アプリは任意の Slint トップレベル コンポーネントをこのトレイトで包み、
/// [`child::open_child_modal`] へ渡す。ウィンドウの表示・モーダル化・中央配置・
/// Esc/Enter/閉じるボタンの読み替え・破棄はライブラリが行い、OK / キャンセルを
/// [`accept`](ModalChild::accept) / [`cancel`](ModalChild::cancel) として
/// ダイアログへ返してくる。
pub trait ModalChild: 'static {
	/// このダイアログの Slint ウィンドウ ハンドル。
	fn window(&self) -> &slint::Window;
	/// 表示する。
	fn show(&self) -> Result<(), slint::PlatformError>;
	/// 隠す (破棄の前段)。
	fn hide(&self) -> Result<(), slint::PlatformError>;
	/// 確定 (Enter / OK ボタン) をダイアログへ伝える。
	fn accept(&self);
	/// キャンセル (Esc / 閉じるボタン / CloseRequested) をダイアログへ伝える。
	fn cancel(&self);
}

/// メインウィンドウ内オーバーレイ モーダルのホスト側抽象。
///
/// アプリはメインウィンドウのオーバーレイ UI (暗転 + 中央パネル) をこの
/// トレイトで包む。表示の切替と OK / キャンセルの転送だけをアプリが書き、
/// 二重オープンガードや Esc/Enter の読み替えはライブラリが行う。
pub trait OverlayHost {
	/// オーバーレイが表示中か。
	fn is_open(&self) -> bool;
	/// オーバーレイを表示する。
	fn open(&self);
	/// オーバーレイを隠す。
	fn close(&self);
	/// 確定 (Enter / OK ボタン) 時の処理 (保存など)。
	fn accept(&self);
	/// キャンセル (Esc / キャンセル ボタン) 時の処理。
	fn cancel(&self);
	/// 表示直前の準備 (現在値のダイアログへの同期など)。
	fn will_open(&self) {}
}

/// オーバーレイ モーダルを開く (二重オープン ガード付き)。
///
/// 子ウィンドウ モードでは何もしない (定数畳み込みで呼び出し全体が消える)。
pub fn open_overlay(host: &dyn OverlayHost) {
	if !MODAL_MODE.is_in_window_overlay() {
		return;
	}
	if host.is_open() {
		return;
	}
	host.will_open();
	host.open();
}

/// オーバーレイ モーダルを閉じる。
pub fn close_overlay(host: &dyn OverlayHost) {
	if !MODAL_MODE.is_in_window_overlay() {
		return;
	}
	host.close();
}

/// メインウィンドウの winit イベント フックから呼ぶ。
///
/// オーバーレイ表示中は Esc = [`cancel`](OverlayHost::cancel) /
/// Enter = [`accept`](OverlayHost::accept) をホストへ転送し、それ以外のキーも
/// 含めてすべて拾う (下の UI に届かないようにする)。戻り値が `true` のとき
/// 呼び出し側は `EventResult::PreventDefault` を返してイベントを破棄すること。
/// オーバーレイ非表示中 / 子ウィンドウ モードでは `false` を返し、
/// アプリ本来のキー処理を続行させる。
pub fn overlay_key_filter(
	host: &dyn OverlayHost,
	code: slint::winit_030::winit::keyboard::KeyCode,
	repeat: bool,
) -> bool {
	if !MODAL_MODE.is_in_window_overlay() {
		return false;
	}
	if !host.is_open() {
		return false;
	}
	if !repeat {
		use slint::winit_030::winit::keyboard::KeyCode;
		match code {
			KeyCode::Escape => host.cancel(),
			KeyCode::Enter | KeyCode::NumpadEnter => host.accept(),
			_ => {}
		}
	}
	true
}
