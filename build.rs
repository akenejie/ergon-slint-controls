use std::path::PathBuf;

fn main() {
	// ライブラリ本体 (as_library)。アプリの生成コードが @ergon_slint_controls
	// import を解決するときに使う SLINT_LIBRARY_* メタデータは、これが
	// cargo::metadata として吐き、依存アプリの build.rs へ DEP_* 環境変数と
	// して届く。
	slint_build::compile_with_config(
		"ui/lib.slint",
		slint_build::CompilerConfiguration::new().as_library("ergon_slint_controls"),
	)
	.unwrap();

	// サンプル (じかん君) も外部アプリと同じ from_library 経路でコンパイルす
	// る。サンプルは本クレート自身なので cargo は DEP_* を運んでくれないた
	// め、同じ名前の環境変数を自前で設定してからコンパイルする (typeloader
	// はこの環境変数を読んで @ergon_slint_controls import をライブラリの
	// SharedGlobals 経由にする = MenuBarTheme の実体がライブラリへ通知され、
	// Linux では GTK テーマ色が反映される)。
	let manifest_dir =
		PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));
	let library_source = manifest_dir.join("ui").join("lib.slint");
	std::env::set_var("DEP_ERGON_SLINT_CONTROLS_SLINT_LIBRARY_NAME", "ergon_slint_controls");
	std::env::set_var(
		"DEP_ERGON_SLINT_CONTROLS_SLINT_LIBRARY_PACKAGE",
		"ergon-slint-controls",
	);
	std::env::set_var(
		"DEP_ERGON_SLINT_CONTROLS_SLINT_LIBRARY_SOURCE",
		library_source.to_string_lossy().as_ref(),
	);
	slint_build::compile_with_config(
		"ui/timekeeper.slint",
		slint_build::CompilerConfiguration::new(),
	)
	.unwrap();
}
