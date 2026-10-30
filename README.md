# ergon-slint-controls

Slint 向けの汎用部品集と OS 統合ユーティリティ。デスクトップ GUI アプリに必要な
「メニューバー + モーダル ダイアログ + ネイティブ メッセージ ボックス +
Windows 風の `&` アクセスキー」を 1 つのライブラリで揃える。

配置や WM 制御は OS / WM の責任とし、このライブラリは「部品の描画」「記法の
解釈」「OS ネイティブ機能の呼び出し」までを担い、それをどう使うかはすべて
呼び出し側に任せる。

| 柱 | 内容 | 参照方法 |
|---|---|---|
| Slint 部品 | メニューバー / 小ボタン / トグルボタン / チェックボックス / スピンボックス / ラベル / ツールチップ | `.slint` 内で `import { ... } from "@ergon_slint_controls"` |
| Rust ユーティリティ | `menu` (`&` 記法パース) / `dialogs` (ネイティブ ダイアログ) / `platform` (タイトルバー・アイコン・修飾キー) / `cursor` (カーソル制御) / `gtk` (Linux) | `use ergon_slint_controls::...` |
| modal-window | モーダル ダイアログ制御ライブラリ (同梱サブクレート) | `use modal_window::...` |

**完全に動くサンプルが `examples/timekeeper.rs`** (作業時間トラッカー「じかん君」)。
全機能を実際に配線したコードはここにある。この README は API の解説であり、
ライブラリに触る要点だけを載せる。

```sh
cargo run --example timekeeper
```

(サンプルは左下ポップアップ方式 + `MenuDismissLayer` による外側クリック閉じ。
ステータスバー方式は `MenuHintHub.text` を自分のステータス行に束ねる。)

---

## 参照方法の要点

### Cargo.toml

```toml
[dependencies]
# slint 本体。ergon-slint-controls と同じ feature セットにする。
slint = { version = "1.17", features = ["std", "backend-winit", "renderer-femtovg", "compat-1-2", "unstable-winit-030"] }
# バックエンドも明示 (modal_window::prepare_backend が生成属性フックを仕込む)。
i-slint-backend-winit = { version = "1.17.1", features = ["renderer-femtovg"] }
# 本ライブラリ (バージョンは git タグで固定)。
ergon-slint-controls = { git = "https://github.com/akenejie/ergon-slint-controls", tag = "v0.3.43" }
# モーダル制御 (同梱サブクレート)。方式は feature で固定できる。
modal-window = { git = "https://github.com/akenejie/ergon-slint-controls", tag = "v0.3.43", features = ["force-child-window"] }

[build-dependencies]
# experimental-module-builds は必須 (下の @import 解決の仕組み)。
slint-build = { version = "1.17", features = ["experimental-module-builds"] }

# modal-window は生 winit のパッチ フォークを必要とする。
# Cargo の仕様で [patch] はビルドのルート (アプリ) に置く。
[patch.crates-io]
winit = { git = "https://github.com/akenejie/slint-modal-winit", tag = "v0.30.13-modal.3" }
```

Linux は `libgtk-3-dev` + `pkg-config` が必要 (同一プロセス GTK ダイアログ用)。

### build.rs

```rust
fn main() {
    let config = slint_build::CompilerConfiguration::new().with_style("fluent-light".into());
    slint_build::compile_with_config("ui/main.slint", config).unwrap();
}
```

`.slint` 内の `@ergon_slint_controls` import は、本ライブラリの
`links = "ergon-slint-controls"` が build.rs へ届ける
`DEP_ERGON_SLINT_CONTROLS_SLINT_LIBRARY_*` メタデータで自動解決される
(`as_library` コンパイル + `experimental-module-builds`)。build.rs には書かない。

### .slint (マークアップ側の参照)

```slint
import { Label, SmallButton, ModeToggleButton, CosmicCheckBox, SpinBox,
         FluentMenuBarImpl, FluentMenuBarItem, FluentMenuFrame, MenuItemView,
         MenuAccessKey, AccessKeyHub, AppTooltip } from "@ergon_slint_controls";

// グローバルを Rust 側から触るにはルート .slint で再エクスポートする (slint の仕様)
export { MenuAccessKey, AccessKeyHub }
```

### Rust (ライブラリ側の参照)

```rust
use ergon_slint_controls::menu::{parse_access_key, access_key_char, pressed_char};
use ergon_slint_controls::{AccessKeyHub, AccessKeyParts, MessageIcon, ...};
use modal_window::{ModalSlot, ModalChild, OverlayHost, ...};
```

---

## `&` アクセスキー記法

タイトル文字列に Win32 標準の記法をそのまま書く:

| 記法 | 意味 |
|---|---|
| `&X` | `X` をアクセスキーにする (下線が引かれる) |
| `&&` | リテラルの `&` |
| 末尾の孤立した `&` | リテラルの `&` |
| `&` なし | アクセスキーなし |

**タイトル文字列がアクセスキーの唯一の情報源**。描画 (下線付きラベル) も
Rust 側のキー発火も同じ 1 本の文字列から同じ関数で読むため、表示と動作が
必ず一致する。

---

## Slint 部品

### メニュー (FluentMenuBarImpl / FluentMenuBarItem / FluentMenuFrame)

| 部品 | 主なプロパティ / コールバック | 役割 |
|---|---|---|
| `FluentMenuBarImpl` | (子に `FluentMenuBarItem` を並べる) | メニューバー |
| `FluentMenuBarItem` | `entry: MenuEntry` / `menu-open` / `access-key-underline` / `description` / `clicked()` / `hovered()` / `hover-description` | バー項目。`description` は hover 中に `MenuHintHub` へ報告される |
| `FluentMenuFrame` | `items: [MenuItemView]` / `current-row` / `descriptions: [string]` / `access-key-underline` / `activated(int)` / `content-width` / `content-height` / `layout-min-width` | ドロップダウン本体。`descriptions` は行ごとの説明 (hover / キーボード選択中の行が `MenuHintHub` へ報告される) |
| `MenuItemView` | `entry: MenuEntry` / `shortcut-text: string` | 項目 1 行分 |
| `MenuEntry` | `title` / `shortcut` / `checkable` / `checked` / `enabled` / `is-separator` | 項目データ。`&` 記法は `title` に書く |
| `AccessKeyLabel` | `text` / `underline` / `color` / `font-size` / `font-weight` / `horizontal-alignment` / `plain-text` | `&` 記法を解釈するテキストの素 |
| `MenuHintPopup` | (配置するだけ) | メニュー項目の説明を出す左下ポップアップ (後述) |
| `MenuDismissLayer` | `active` / `top` / `dismiss()` | メニューの外側クリックで閉じるレイヤ (後述) |

ドロップダウンは標準 `MenuBar` と違い **アプリが配置する in-window オーバーレイ**:

```slint
file-popup := FluentMenuFrame {
    visible: MenuOpenHub.open == 0;             // 開閉状態はハブが持つ
    x: file-button.absolute-position.x;
    y: menu-bar.height - 1px;
    width: max(file-popup.content-width, file-popup.layout-min-width);
    height: file-popup.content-height;
    items: root.file-items;
    access-key-underline: root.menu-alt-held;   // Alt 押下中だけ下線
    current-row <=> root.menu-index;            // hover とキーボード選択の集約点
    activated(i) => {
        MenuOpenHub.close();                    // 選んだら閉じる
        /* 行 i の処理 */
    }
}
```

**キーボード操作 (開閉 / 上下 / Enter / アクセスキー) はライブラリが提供しない。**
項目モデルが有効状態の唯一の情報源なので、アプリはモデルを読んで区切り行と
無効行を読み飛ばしながら `current-row` を動かす。パターンは
examples/timekeeper.rs のキー処理 (Alt+文字で開閉、ArrowUp/Down、Enter、Esc、
開閉中のアクセスキー発火) をそのまま使える。

### 部品

| 部品 | 主なプロパティ | コールバック | 備考 |
|---|---|---|---|
| `Label` | `text` (`&` 記法、下線は常に表示) | `access-key-pressed()` | アクセスキーが押されると発火 |
| `SmallButton` | `text` / `icon` / `icon-size` / `tooltip-text` / `access-key-underline` / `primary` / `enabled` / `checkable` / `colorize-icon` | `clicked()` | 高さ 22px 級の小ボタン |
| `ModeToggleButton` | 同上 | `clicked()` | 群から 1 つ選ぶトグル。`checked` で見た目が変わる |
| `CosmicCheckBox` | `checked` / `checked-color` | (なし) | 見た目だけのチェックボックス。クリック処理は自前実装 |
| `SpinBox` | `value` (in-out) / `minimum` / `maximum` / `step-size` / `read-only` / `enabled` / `tooltip-text` / `horizontal-alignment` / `has-focus` | `edited(int)` | 上下ボタン縦並び。数値の直接入力も可 |
| `SpinBoxBase` / `SpinBoxButton` | フォント / 選択色 / `icon-width` / `icon-height` | `update-value()` / `increment()` / `decrement()` / `clicked()` | 上級者向けの内部部品 |
| `AppTooltip` | `text` / `padding-h` / `padding-v` | (なし) | 組み込み `Tooltip` 要素 (slint 本体の機能) の中身として使う |

### グローバル

| グローバル | メンバー | 役割 |
|---|---|---|
| `MenuAccessKey` | `pure callback parse(title) -> AccessKeyParts` | 描画側の `&` 記法解釈。**アプリは `on_parse` で Rust 実装へ差し込む (必須)** |
| `AccessKeyHub` | `press(key: string)` / `pressed` | ラベルのアクセスキー通知ハブ |
| `MenuHintHub` | `text` / `frame-text` / `bar-text` | メニュー項目の説明 (ツールチップ) を集めるハブ (後述) |
| `MenuOpenHub` | `open` / `close()` / `toggle(index)` | メニューの開閉状態の唯一の情報源。外側クリック捕捉レイヤもこれを読む |

### メニュー項目の説明 (ツールチップ)

メニューの項目に説明を付けると、hover またはキーボード選択中にその説明が
`MenuHintHub.text` に集まる。バー項目は `description` プロパティ、ドロップ
ダウンは `descriptions` プロパティ (行ごとの配列) で渡す:

```slint
file-button := FluentMenuBarItem {
    entry: { title: root.file-menu-title };
    description: "CSV の読み込みや出力を行います。";   // hover 中にハブへ報告
    ...
}
file-popup := FluentMenuFrame {
    items: root.file-items;
    descriptions: [ "フォルダを選びます。", "CSV に書き出します。", "", "終了します。" ];
    ...
}
```

**出力先は選べる**:

```slint
// (a) 左下ポップアップ (ステータスバーのないアプリ。ウィンドウのルートの
//     最後に置く。hover 中だけ左下に浮かぶ)
MenuHintPopup { }

// (b) ステータスバーのあるアプリ: 自分のステータス行に束ねる
//     (見た目はアプリ固有なのでライブラリは描かない)
Text { text: MenuHintHub.text != "" ? MenuHintHub.text : root.own-hint; }
```

優先順位は「開いているドロップダウンの項目の説明 > バー項目の説明」
(メニューを開いている間は項目の説明、閉じている間はボタンの説明が出る)。

### メニューの開閉と外側クリック / 非活性で閉じる

メニューの開閉状態は `MenuOpenHub` グローバルが唯一の情報源。バー項目と
ポップアップを束ね、外側クリックはライブラリが閉じる:

```slint
file-button := FluentMenuBarItem {
    entry: { title: root.file-menu-title };
    menu-open: MenuOpenHub.open == 0;
    hovered => { if (MenuOpenHub.open >= 0) { MenuOpenHub.open = 0; } }  // 開き替え
    clicked => { MenuOpenHub.toggle(0); }
}

// in-window オーバーレイのドロップダウンは、開いている間の外側クリックを
// ライブラリが捕捉して閉じる (メニューバーより下だけ覆うので、バー項目の
// hover / 開き替えは生きる。押した瞬間 = down で閉じ、全ボタン共通)。
// active は既定で MenuOpenHub に追従する。**配置はドロップダウンより手前
// (先に宣言)** — 後ろに置くとポップアップ項目の hover / クリックを塞ぐ:
MenuDismissLayer { top: menu-bar.height; }
```

**ウィンドウの外がクリックされた (非活性になった) ときの閉じ**は、winit フック
から 1 行ハブへ伝える (フォーカスイベントの観測だけがアプリ側の責任):

```rust
if let winit::event::WindowEvent::Focused(false) = event {
    ui.global::<MenuOpenHub>().invoke_close();
}
```

---

## Rust ユーティリティ

### `menu` — `&` 記法のパース

```rust
// 差し込み (main で一度。AccessKeyParts はライブラリがエクスポートする型):
ui.global::<MenuAccessKey>().on_parse(|title| {
    let p = parse_access_key(&title);
    ergon_slint_controls::AccessKeyParts {
        before: p.before.into(), key: p.key.into(), after: p.after.into(),
    }
});
```

- `parse_access_key(&str) -> AccessKeyParts` — `{ before, key, after }`。`key` が
  空ならアクセスキーなし
- `access_key_char(&str) -> Option<char>` — `&X` の X。Rust 側のキー発火はこれで
  タイトルと比較する
- `pressed_char(&winit::event::KeyEvent) -> Option<char>` — 押されたキーの文字
  (小文字)。**離し (Released) は無視**するので、押下と離しの二重発火がない。
  winit フックのキー処理はこれ 1 本でよい

### ラベルのアクセスキー → 任意の処理

`.slint` 側:

```slint
Label {
    text: root.break-title;                       // "休憩の合図 (&R):"
    access-key-pressed => { spin.focus(); }       // 任意の処理は呼び出し側
}
```

Rust 側 (キー処理からハブへ通知すると、一致する Label が発火する):

```rust
ui.global::<AccessKeyHub>().invoke_press(pressed.to_ascii_uppercase().to_string().into());
```

ボタン (トグルボタンを含む) 用のアクセスキー機構は slint に無いため、
`access_key_char(ボタンのタイトル)` をキー処理で読んで callback を発火させる
(例は examples/timekeeper.rs の「開始 / 停止ボタンのアクセスキー」)。

### `dialogs` — ネイティブ ダイアログ

| 関数 | シグネチャ | 挙動 |
|---|---|---|
| `show_message_box` | `(title, text, kind: MessageIcon)` | エラー / 情報。fire-and-forget。Windows = TaskDialog、Linux = 同一プロセス GTK (親をモーダルにブロック)、macOS = NSAlert。失敗時は zenity / kdialog / xmessage、最後は stderr。**1 フレーム遅らせて表示する** (メニューを閉じた直後に呼んでも、閉じた描画が先に行われポップアップが残らない) |
| `message_box` | `(title, text, buttons: MessageButtons) -> Option<MessageResponse>` | `Ok` / `OkCancel` / `YesNo` / `YesNoCancel`。キャンセル / Esc は `None` |
| `message_box_custom` | `(title, text, choices: &[&str], default: Option<usize>) -> Option<usize>` | 選択 index。ラベル中の `(&X)` は各 OS のアクセスキー表記に変換 |
| `prompt_folder` | `(title, initial: Option<&Path>) -> Option<PathBuf>` | フォルダ選択。Windows = IFileOpenDialog、macOS = NSOpenPanel |
| `capture_dialog_owner` | `(ui: &slint::Window)` | Windows: メイン ウィンドウ HWND を owner として確保。winit フックで 1 回呼ぶ |
| `dialog_active` / `stale_input_pending` | `() -> bool` | ダイアログ表示中 / 閉じた直後に入力がまだ残っているか |
| `dialog_push` / `dialog_pop` | `()` | 自前の入力遮断区間を作るときのカウンタ |

`MessageIcon` = `Error | Info`。`MessageButtons` =
`Ok | OkCancel | YesNo | YesNoCancel`。`MessageResponse` = `Ok | Cancel | Yes | No`。

ダイアログが OS のモーダリティで守られない環境 (別プロセス表示など) の保険と
して、表示中と閉じ直後は winit フックで親の入力を握り潰す
(examples/timekeeper.rs の「ネイティブ ダイアログが表示中 …」ブロック参照):

```rust
if dialog_active() || stale_input_pending() { return EventResult::PreventDefault; }
```

### `platform` — OS 固有の窓周り

| 関数 | OS | 内容 |
|---|---|---|
| `apply_titlebar_theme(&ui, TitlebarTheme::Light / Dark / System)` | Windows / macOS | 装飾テーマの指定。内部 UI を fluent-light に固定するなら Light を明示 (指定しないと UI と装飾の明暗が分裂する)。モーダル子ダイアログは modal-window が既定で親に揃える |
| `applied_titlebar_theme()` | 同上 | 現在適用済みのテーマ |
| `install_window_icon(hwnd, resource_id)` | Windows | exe 埋め込み .ico をタイトルバー / タスクバーへ設定 |
| `main_window_hwnd(&slint::Window)` | Windows | HWND を取得 |
| `os_modifier_keys()` | 全 OS | `(shift, ctrl)` の押下状態 (macOS の物理差異を吸収) |
| `encode_wide(&str)` | Windows | UTF-16 変換 (Win32 API を直接呼ぶアプリ用の補助) |

### `cursor` — カーソル制御

| 関数 | 内容 |
|---|---|
| `cache_window(Arc<winit::window::Window>)` | 対象 winit ウィンドウを渡す (`ui.window().winit_window().await` で取得) |
| `set_system_cursor(SystemCursor)` | `Default` / `Crosshair` |
| `set_custom_cursor(key, || CursorBitmap) -> bool` | 自前カーソル。`CursorBitmap { width, height, hotspot_x, hotspot_y, pixels: Vec<u32> }` (0xAARRGGBB、行優先) |
| `set_file_cursor(key, &Path) -> bool` | `.cur` ファイルから取り付け (Windows) |
| `install_override(&slint::Window)` | Windows: `WM_SETCURSOR` でも自前カーソルを維持するようサブクラス化 |
| `take_non_client_click() -> bool` | Windows: 非クライアント領域をクリックしたか |

### `gtk` (Linux のみ)

同一プロセス GTK ダイアログの低レイヤ。通常は `dialogs` が内部で使う
(`show_message_box` の Linux 経路がこれ):

| 関数 | 内容 |
|---|---|
| `gtk_ready() -> bool` | GTK 初期化に成功したか |
| `run_dialog_blocking(&dlg) -> ResponseType` | GtkDialog を親をブロックして表示し、後始末 (グラブ解除 / 破棄 / GDK flush / X フォーカス復帰) までを統一して行う。**自前で GtkDialog を出すときは必ずこれを使う** (flush と破棄を省くと、閉じた後に X の入力フォーカスが死ぬ) |
| `add_button(&dlg, group, label, response)` | ボタン追加 (ラベル中の `(X)` を Alt+X アクセラレータに) |

---

## modal-window — モーダル ダイアログ制御

Slint ウィンドウを「モーダル子ダイアログ」として出す仕掛け。ダイアログも
ウィンドウ マネージャの管理下に置き、所有ウィンドウ / ダイアログ型という
OS 標準のヒントでモーダル性を伝える。WM がヒントを無視する環境でも、
ライブラリ側が親の入力遮断を担うため「親より前面 + 親を操作不可」は保たれる。

| 方式 | 向き | 選び方 |
|---|---|---|
| `ChildWindow` | 別 OS ウィンドウの子ダイアログ (デスクトップ向き) | feature `force-child-window` |
| `InWindowOverlay` | メイン ウィンドウ内の暗転オーバーレイ + パネル (モバイル / 車載向き) | feature `force-in-window-overlay` |

既定は OS ごと (`MODAL_MODE.default_for_os()`: Windows = 子ウィンドウ、それ以外 =
オーバーレイ)。分岐は `MODAL_MODE.is_in_window_overlay()` (const) で問い合わせる。

### 子ウィンドウ方式の呼び出し順

```rust
// 1. main() の最初でバックエンドを準備 (Windows: ダイアログ枠 + 所有ウィンドウ、
//    X11: _NET_WM_WINDOW_TYPE_DIALOG の生成属性フックを仕込む)
modal_window::prepare_backend()?;

// 2. スロットを用意 (開いていれば Some、閉じたら None を管理)
let slot: modal_window::ModalSlot = Default::default();

// 3. 開く: ダイアログのラッパ型に ModalChild (window / show / hide / accept /
//    cancel の 5 メソッド) を実装しておく
#[cfg(any(windows, all(unix, not(target_os = "macos"))))]
modal_window::mark_next_window_as_child_dialog();   // 次のウィンドウ生成が子ダイアログ
modal_window::open_child_modal(&slot, &main, Rc::new(ExportDialog(dlg)));
//    表示・Esc/Enter の読み替え (accept/cancel へ転送)・中央配置・破棄はライブラリ

// 4. 閉じる (OK / キャンセル ハンドラから。hide ではなく破棄される)
modal_window::close_child_modal(&slot, main.window());

// 5. Windows: メイン ウィンドウ生成後に owner を登録 (1 回でよい)
modal_window::note_owner_window(winit_window);
```

中央配置の決定責任は OS の文化に従う: Windows は OS が配置しないため winit
フォークが表示直前に owner 中央へ配置し、Linux は WM が transient を親基準に
配置する (ライブラリは座標を扱わない)。

### オーバーレイ方式の呼び出し順

```rust
// パネルのホスト。表示の切り替えと確定 / キャンセルの転送だけを書く:
impl modal_window::OverlayHost for ExportPanelHost {
    fn is_open(&self) -> bool { /* パネル表示中か */ }
    fn open(&self) { /* パネルを表示 */ }
    fn close(&self) { /* パネルを隠す */ }
    fn accept(&self) { /* 確定 (Enter / OK) */ }
    fn cancel(&self) { /* キャンセル (Esc) */ }
    fn will_open(&self) {}   // 省略可
}
modal_window::open_overlay(&host);     // 開く (二重オープン ガード付き)
modal_window::close_overlay(&host);    // 閉じる
```

Esc / Enter の転送は winit フックから:

```rust
if modal_window::overlay_key_filter(&*host, code, key_event.repeat) {
    return EventResult::PreventDefault;   // オーバーレイが消費した
}
```

`ModalSession` / `WeakModalSlot` / `dismiss_modal_dialog` / `enforce_modal_styles`
は高度な制御用 (自前でライフサイクルを管理する場合)。

---

## キーボード処理の土台

キー処理は slint の winit フックで行う (`unstable-winit-030`):

```rust
use slint::winit_030::{winit, EventResult, WinitWindowAccessor};

ui.window().on_winit_window_event(move |_win, event| -> EventResult {
    if let winit::event::WindowEvent::ModifiersChanged(mods) = event {
        // Alt 押下中はメニューのアクセスキーに下線を表示する
        ui.set_menu_alt_held(mods.state().alt_key());
    }
    if let winit::event::WindowEvent::KeyboardInput { event: key_event, .. } = event {
        // physical_key: KeyCode / logical_key: 押された文字
    }
    EventResult::Propagate
});
```

フル実装 (Alt+文字でメニュー開閉、有効行だけを辿る上下移動、Enter / Esc、
開閉中の項目アクセスキー、ラベルとボタンのアクセスキー) は
**examples/timekeeper.rs** を参照。

## バージョンの固定と更新

- `Cargo.toml` の `tag = "v..."` でバージョンを固定する
- 更新するときは **タグの書き換え + `cargo update -p ergon-slint-controls`**
  (ロックの更新を忘れると、タグを変えても古いビルドのままになる)
- modal-window も同じタグに揃える。winit パッチ フォークのタグは
  modal-window の要求に従う

## ライセンス

Slint の公式ウィジェット (menu-base / menu / fluent) を改変して含む部分は
SPDX 表記のとおり:
`GPL-3.0-only OR LicenseRef-Slint-Royalty-free-2.0 OR LicenseRef-Slint-Software-3.0`。
それ以外の部分はリポジトリのライセンス ファイルを参照。
