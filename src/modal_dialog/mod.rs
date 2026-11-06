//! モーダル ダイアログ制御ライブラリ。
//!
//! アプリは「ダイアログの中身」と「開く/閉じる指示」だけを書き、OS 間の差分
//! (子ウィンドウの生成属性・タイトルバー整え・テーマの親への追従・Esc/Enter
//! の読み替え・親ウィンドウの入力遮断・破棄) はこのクレートが担う。配置は
//! OS 間で「位置の決定責任」が異なる点にも注意:
//!
//! - **Linux**: WM が transient ウィンドウを親基準に配置する文化があるため、
//!   ライブラリは座標を扱わない (`WM_TRANSIENT_FOR` を伝えるだけ)
//! - **Windows**: OS はダイアログの位置を決めない (既定はカスケード等)。
//!   したがって位置を決める必要があるが、生成時にはまだサイズが確定しない
//!   (slint は表示直前にリサイズする)。そこで winit パッチ フォークの
//!   `with_dialog_center_over_owner` を指示し、**フォークが表示直前
//!   (`ShowWindow` の直前) に所有者の中央へ配置**する。実矩形で計算するため
//!   最初のフレームから正位置で、生成後の移動によるちらつきはない
//! - macOS: 未検証 (OS のカスケード配置に従う)
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
//! # 表示方式 (アプリ側の定数で決める)
//!
//! このモジュールは子ウィンドウ方式とオーバーレイ方式の**両方の実装**を含み、
//! バイナリへ含めるかどうかだけが Cargo の feature `modal-dialog` で決まる。
//! どちらの方式でダイアログを出すかはライブラリ側の設定ではなく、**アプリの
//! ソースコード内にフラグとなる定数を書いて**決める:
//!
//! ```rust
//! use ergon_slint_controls::modal_dialog::ModalMode;
//!
//! /// このアプリのモーダル表示方式 (全 OS で子ウィンドウに固定する例)
//! const MODAL_MODE: ModalMode = ModalMode::ChildWindow;
//! ```
//!
//! OS で分けたければ、アプリ側で `cfg!` してよい (ライブラリは既定値を
//! 持たない。方式の選択は完全にアプリ側の判断):
//!
//! ```rust
//! # use ergon_slint_controls::modal_dialog::ModalMode;
//! // Windows だけ子ウィンドウ、それ以外はオーバーレイにする例
//! const MODAL_MODE: ModalMode = if cfg!(windows) {
//! 	ModalMode::ChildWindow
//! } else {
//! 	ModalMode::InWindowOverlay
//! };
//! ```
//!
//! - [`ModalMode::ChildWindow`] = 全 OS で子ウィンドウ
//! - [`ModalMode::InWindowOverlay`] = 全 OS でウィンドウ内オーバーレイ
//!
//! アプリ側の分岐はこの定数に対して行う
//! (`if MODAL_MODE.is_in_window_overlay() { ... }`)。定数はアプリのクレート内
//! にあるため分岐はコンパイル時に畳み込まれ、release ビルド (LTO) では選ばれ
//! なかった側の実装も dead-code elimination で除去される。ライブラリの各関数
//! は方式を知らない (開く / 閉じるだけ)。子ウィンドウ方式の API
//! ([`backend::prepare_backend`] / [`child::open_child_modal`] 等) と
//! オーバーレイ方式の API ([`open_overlay`] 等) の呼み分けはアプリの責任で、
//! その分岐に同じ定数を使う。
//!
//! # 子ダイアログのスタイル (アイコン無しのダイアログ枠) — winit フォーク必須
//!
//! モーダル子ダイアログの慣習的なスタイルは「アイコン無しのダイアログ枠」
//! (Windows: `WS_EX_DLGMODALFRAME`)。**Windows はウィンドウ生成後に
//! タイトルバーのアイコンを消せない** (生成後の Win32 修正は winit のスタイル
//! 再計算で戻される)。この教訓から、本モジュールは子ダイアログを生成時点から
//! 正しい属性で作るため winit パッチ フォーク
//! (https://github.com/akenejie/slint-modal-winit) を**必須**としている:
//!
//! - Windows: `WindowAttributes::with_dialog_modal_frame` で生成時から
//!   `WS_EX_DLGMODALFRAME` (アイコン無しのダイアログ枠)。`WindowFlags` として
//!   保持されるため winit のスタイル再計算でも外れない
//! - X11: `Window::set_transient_for` / `set_modal` で
//!   `WM_TRANSIENT_FOR` / `_NET_WM_STATE_MODAL` を付ける
//!   (`_NET_WM_WINDOW_TYPE_DIALOG` は上流 API で生成時に付与)
//!
//! このフォークは `modal-dialog` feature を有効にしたときだけ必要になる
//! (`default` は feature 無効なので、メニュー / ネイティブダイアログだけを使う
//! アプリは fork を要求されない)。Cargo の `[patch.crates-io]` は**ビルドの
//! ルート (アプリ) の manifest でしか有効にならず、ライブラリから消費側へ伝播
//! しない** (Cargo の仕様)。したがって feature を有効にするアプリは、自分の
//! Cargo.toml に次の 1 行を置く:
//!
//! ```toml
//! [patch.crates-io]
//! winit = { git = "https://github.com/akenejie/slint-modal-winit", tag = "v0.30.13-modal.10" }
//! ```
//!
//! 本リポジトリ (ergon-slint-controls) のワークスペースも同じパッチを持ち、
//! 自身のビルド/テストはフォーク前提で行う。フォークは upstream winit
//! v0.30.13 を基準にしたコミット列なので、差分は `git diff v0.30.13..main`
//! で見え、上流追従は merge で行える。
//!
//! # 起動時の準備 (子ウィンドウ方式のときに呼ぶ)
//!
//! [`backend::prepare_backend`] を `main` 冒頭で呼ぶと、以降
//! [`backend::mark_next_window_as_child_dialog`] の直後に生成される Slint
//! ウィンドウが「親のモーダル子ダイアログ」用の OS 属性 (Windows: ダイアログ枠
//! + 所有ウィンドウ / X11: `_NET_WM_WINDOW_TYPE_DIALOG`) 付きで作られる。
//! オーバーレイ方式では子ウィンドウを生成しないため、呼ぶ必要はない
//! (呼んでもフックは mark_* で予約が立つまで働かないため害もない)。

pub mod backend;
pub mod child;

pub use backend::{mark_next_child_dialog_of, mark_next_window_as_child_dialog, prepare_backend};
#[cfg(windows)]
pub use backend::{note_owner_hwnd, note_owner_window};
pub use child::{
	close_child_modal, dismiss_modal_dialog, enforce_modal_styles,
	open_child_modal, ModalSession, ModalSlot, WeakModalSlot,
};

// --- 表示方式 (アプリ側の定数が選ぶ。ライブラリは方式を知らない) ---

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

/// 子ウィンドウ (別 OS ウィンドウ) モーダルのダイアログ側抽象。
///
/// アプリは任意の Slint トップレベル コンポーネントをこのトレイトで包み、
/// [`child::open_child_modal`] へ渡す。ウィンドウの表示・モーダル化・テーマの
/// 親への追従・Esc/Enter/閉じるボタンの読み替え・破棄はライブラリが行い、
/// OK / キャンセルを [`accept`](ModalChild::accept) / [`cancel`](ModalChild::cancel)
/// としてダイアログへ返してくる。
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
/// オーバーレイ方式の API。子ウィンドウ方式のアプリから呼ばないこと。
pub fn open_overlay(host: &dyn OverlayHost) {
	if host.is_open() {
		return;
	}
	host.will_open();
	host.open();
}

/// オーバーレイ モーダルを閉じる。
///
/// オーバーレイ方式の API。子ウィンドウ方式のアプリから呼ばないこと。
pub fn close_overlay(host: &dyn OverlayHost) {
	host.close();
}

/// モーダル中の Esc / Enter を確定・キャンセルへ読み替える共通処理。
/// 子ウィンドウのイベントフックとオーバーレイのキー フィルタで共有する。
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum ModalKeyAction {
	Accept,
	Cancel,
}

pub(crate) fn modal_key_action(
	code: slint::winit_030::winit::keyboard::KeyCode,
) -> Option<ModalKeyAction> {
	use slint::winit_030::winit::keyboard::KeyCode;
	match code {
		KeyCode::Escape => Some(ModalKeyAction::Cancel),
		KeyCode::Enter | KeyCode::NumpadEnter => Some(ModalKeyAction::Accept),
		_ => None,
	}
}

/// メインウィンドウの winit イベント フックから呼ぶ。
///
/// オーバーレイ表示中は Esc = [`cancel`](OverlayHost::cancel) /
/// Enter = [`accept`](OverlayHost::accept) をホストへ転送し、それ以外のキーも
/// 含めてすべて拾う (下の UI に届かないようにする)。戻り値が `true` のとき
/// 呼び出し側は `EventResult::PreventDefault` を返してイベントを破棄すること。
/// オーバーレイ非表示中は `false` を返し、アプリ本来のキー処理を続行させる。
/// オーバーレイ方式の API。子ウィンドウ方式のアプリから呼ばないこと。
pub fn overlay_key_filter(
	host: &dyn OverlayHost,
	code: slint::winit_030::winit::keyboard::KeyCode,
	repeat: bool,
) -> bool {
	if !host.is_open() {
		return false;
	}
	if !repeat {
		match modal_key_action(code) {
			Some(ModalKeyAction::Cancel) => host.cancel(),
			Some(ModalKeyAction::Accept) => host.accept(),
			None => {}
		}
	}
	true
}
