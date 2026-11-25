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
//! アプリはこの定数を [`ModalController::new`] へ渡すだけでよく、方式ごとの
//! 分岐を書く必要はない。ライブラリが表示面 ([`OverlaySurface`]) か子ウィンド
//! ウ ([`child::open_child_modal`]) かを判断して開閉する。
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

use std::rc::Rc;
use slint::ComponentHandle;

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
	/// こ�Eダイアログの Slint ウィンドウ ハンドル、E
	fn window(&self) -> &slint::Window;
	/// 表示する、E
	fn show(&self) -> Result<(), slint::PlatformError>;
	/// 隠ぁE(破棁E�E前段)、E
	fn hide(&self) -> Result<(), slint::PlatformError>;
	/// 確宁E(Enter / OK ボタン) をダイアログへ伝える、E
	fn accept(&self);
	/// キャンセル (Esc / 閉じる�Eタン / CloseRequested) をダイアログへ伝える、E
	fn cancel(&self);
	/// ウィンドウ イベントの事前処理 (アプリ固有のキー操作など)。モーダル
	/// ウィンドウがイベントを受け取るたび、ergon 既定の処理 (CloseRequested /
	/// Esc / Enter の読み替え) の**前に**呼ばれる。`PreventDefault` を返すと
	/// ergon 既定の処理は行われない (イベントは slint へも渡らない)。
	/// アクセスキー付きラベルの発火など、ダイアログ固有のキー操作に使う。
	fn window_event(&self, _event: &slint::winit_030::winit::event::WindowEvent) -> slint::winit_030::EventResult {
		slint::winit_030::EventResult::Propagate
	}
}

/// オーバーレイ方式の表示面のハブ。slint ライブラリの `ModalOverlayHub`
/// グローバル (表示中のダイアログの key) の実体で、アプリの MainWindow が
/// 1 つ持つ。`ModalController::new` へアプリから渡され、ライブラリが
/// open / close で書き込む (`ModalPopupHost` / `ModalDialogHost` が読む)。
pub type OverlayHub = core::pin::Pin<std::rc::Rc<crate::generated::InnerModalOverlayHub>>;

/// モーダルの開閉を担うハンドル (表示方式をカプセル化する)。
///
/// アプリはダイアログごとに 1 つ作り ([`ModalController::new`])、
/// [`open`](ModalController::open) で開き、OK / キャンセル ハンドラの中で
/// [`close`](ModalController::close) する。表示方式 ([`ModalMode`]) の判断、
/// 二重オープン ガード、子ウィンドウのモーダル スタック ([`ModalSlot`]) は
/// ライブラリが担うため、アプリには方式ごとの分岐もスロット管理も現れない。
#[derive(Clone)]
pub struct ModalController {
	inner: std::rc::Rc<std::cell::RefCell<ControllerInner>>,
}

struct ControllerInner {
	mode: ModalMode,
	/// オーバーレイ方式: 表示面のハブ (メインウィンドウの ModalOverlayHub
	/// グローバルの実体)。
	overlay_hub: OverlayHub,
	/// オーバーレイ方式: このダイアログの key
	/// (子ウィンドウ方式では不使用)。
	key: i32,
	/// 子ウィンドウ方式: ライブラリが所有するモーダル スロット。
	slot: ModalSlot,
}

impl ModalController {
	/// ダイアログごとのコントローラを作る。`overlay_hub` はメインウィンドウの
	/// `ModalOverlayHub` グローバルの実体
	/// (`ui.global::<ModalOverlayHub>().0.clone()`)、`key` はオーバーレイ方式で
	/// このダイアログの表示に使う識別子 (子ウィンドウ方式では不使用)。
	pub fn new(mode: ModalMode, overlay_hub: OverlayHub, key: i32) -> Self {
		Self {
			inner: std::rc::Rc::new(std::cell::RefCell::new(ControllerInner {
				mode,
				overlay_hub,
				key,
				slot: Default::default(),
			})),
		}
	}

	/// モーダルが開いているか。
	pub fn is_open(&self) -> bool {
		let inner = self.inner.borrow();
		match inner.mode {
			ModalMode::InWindowOverlay => {
				crate::generated::ModalOverlayHub(inner.overlay_hub.clone(), core::marker::PhantomData)
					.get_open_key()
					== inner.key
			}
			ModalMode::ChildWindow => inner.slot.borrow().is_some(),
		}
	}

	/// モーダルを開く (二重オープン ガード付き)。
	///
	/// - オーバーレイ方式: `make_child` は呼ばれず、表示面の `key` の
	///   オーバーレイを表示する (ModalOverlayHub.open-key への書き込み)。
	/// - 子ウィンドウ方式: 親ウィンドウへの登録を済ませた上で `make_child` を
	///   呼び、戻り値を子ダイアログとして開く ([`open_child_modal`])。
	///   コントローラは `make_child` へ渡されるので、OK / キャンセル ハンドラの
	///   中から [`close`](Self::close) できる。`make_child` が `None` を返した
	///   (生成に失敗した) ときは何も開かない。
	pub fn open<P: ComponentHandle + 'static>(
		&self,
		parent: &P,
		make_child: impl FnOnce(&ModalController) -> Option<Rc<dyn ModalChild>>,
	) {
		// 二重オープン ガード (inner の borrow の外で判定する)。
		if self.is_open() {
			return;
		}
		let inner = self.inner.borrow();
		match inner.mode {
			ModalMode::InWindowOverlay => {
				inner.overlay_hub.open_key.set(inner.key);
			}
			ModalMode::ChildWindow => {
				// 次のウィンドウ生成がこの親のモーダル子ダイアログだとライブラリ
				// へ伝える (X11 では transient 親も生成時に焼き込む)。
				backend::mark_next_child_dialog_of(parent.window());
				let Some(child) = make_child(self) else { return };
				open_child_modal(&inner.slot, parent, child);
			}
		}
	}

	/// モーダルを閉じる (表示方式によらない。既に閉じていれば何もしない)。
	///
	/// オーバーレイ方式では表示面を隠し、子ウィンドウ方式ではモーダル解除と
	/// 子ウィンドウの破棄を行う。
	pub fn close(&self) {
		let inner = self.inner.borrow();
		match inner.mode {
			ModalMode::InWindowOverlay => {
			crate::generated::ModalOverlayHub(inner.overlay_hub.clone(), core::marker::PhantomData)
				.set_open_key(0);
		}
			ModalMode::ChildWindow => close_child_modal(&inner.slot),
		}
	}
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
/// オーバーレイ表示中は Esc = [`close`](ModalController::close) /
/// Enter = `accept` (確定処理: 適用して閉じるなど) を行い、それ以外のキーも
/// 含めてすべて拾う (下の UI に届かないようにする)。戻り値が `true` のとき
/// 呼び出し側は `EventResult::PreventDefault` を返してイベントを破棄すること。
/// オーバーレイ非表示中は `false` を返し、アプリ本来のキー処理を続行させる。
pub fn overlay_key_filter(
	ctl: &ModalController,
	code: slint::winit_030::winit::keyboard::KeyCode,
	repeat: bool,
	accept: impl FnOnce(),
) -> bool {
	if !ctl.is_open() {
		return false;
	}
	if !repeat {
		match modal_key_action(code) {
			Some(ModalKeyAction::Cancel) => ctl.close(),
			Some(ModalKeyAction::Accept) => accept(),
			None => {}
		}
	}
	true
}
