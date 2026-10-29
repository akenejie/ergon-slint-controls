//! GTK ダイアログの単体動作テスト (デバッグ用)。
//!
//! モード 1 (デフォルト): GTK ダイアログだけをブロック実行する。
//! モード 2 (SLINT=1): slint (winit) イベントループを起動した状態で、
//! UI スレッドから GTK ダイアログをブロック実行する (ikng と同じ構造)。
//!
//! どちらも OK ボタンの絶対座標を stdout へ出力するので、テストスクリプトは
//! そこを正確にクリックできる。

fn main() {
    let with_slint = std::env::var("SLINT").is_ok();
    println!("gtk_ready... (with_slint={with_slint})");
    if !ergon_slint_controls::gtk::gtk_ready() {
        println!("gtk init failed");
        return;
    }
    if !with_slint {
        run_dialog();
        return;
    }
    // slint (winit) イベントループを起動し、1 秒後に UI スレッドで
    // GTK ダイアログをブロック実行する (ikng のメニューと同じ構造)。
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(1));
        let _ = slint::invoke_from_event_loop(run_dialog_and_quit);
    });
    let _ = slint::run_event_loop_until_quit();
}

fn run_dialog_and_quit() {
    run_dialog();
    let _ = slint::quit_event_loop();
}

fn run_dialog() {
    use gtk::prelude::*;
    let dlg = gtk::MessageDialog::new(
        None::<&gtk::Window>,
        gtk::DialogFlags::empty(),
        gtk::MessageType::Info,
        gtk::ButtonsType::Ok,
        "GTK 単体テスト: OK を押してください",
    );
    dlg.set_title("msgbox_test");
    // OK ボタンの絶対座標を定期的に stdout へ出力する (クリックテスト用)。
    let btn = dlg.add_button("OK", gtk::ResponseType::Ok);
    gtk::glib::timeout_add_local(std::time::Duration::from_millis(200), move || {
        let alloc = btn.allocation();
        if let Some(win) = btn.window() {
            let (wx, wy) = win.position();
            println!(
                "OK-POS x={} y={} (w={} h={})",
                wx + alloc.x(),
                wy + alloc.y(),
                alloc.width(),
                alloc.height()
            );
        }
        true.into()
    });
    println!("running dialog (blocking)... click OK");
    let ret = ergon_slint_controls::gtk::run_dialog_blocking(&dlg);
    println!("response: {ret:?}");
    dlg.close();
    println!("done");
}
