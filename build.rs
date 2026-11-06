use std::path::PathBuf;

fn main() {
	embed_windows_manifest();

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

// Windows (MSVC): 実行ファイルにアプリケーション マニフェストを埋め込む。
//
// マニフェストが無いと comctl32.dll が v5.82 に解決され、TaskDialogIndirect
// などの v6 専用エントリ ポイントが見つからず、ロード時に
// STATUS_ENTRYPOINT_NOT_FOUND (0xC0000139) で落ちる (dialogs.rs の TaskDialog
// は静的インポート)。依存アプリは自前でマニフェストを埋め込むことが多い
// (ikng は embed-manifest クレート) が、本クレートのサンプルは単独で動く
// 必要があるためここで埋める。
//
// embed-manifest クレートが吐く cargo:rustc-link-arg-bins は bin にしか効か
// ず examples をカバーしないため、cargo:rustc-link-arg を直接吐く。
fn embed_windows_manifest() {
	let target_os = std::env::var("CARGO_CFG_TARGET_OS").unwrap_or_default();
	let target_env = std::env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
	if target_os != "windows" || target_env != "msvc" {
		return;
	}
	let out = PathBuf::from(std::env::var_os("OUT_DIR").expect("OUT_DIR is set"));
	let manifest = out.join("app-manifest.xml");
	std::fs::write(
		&manifest,
		r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
	<assemblyIdentity name="ergon-slint-controls" type="win32" version="1.0.0.0"/>
	<dependency>
		<dependentAssembly>
			<assemblyIdentity language="*" name="Microsoft.Windows.Common-Controls" processorArchitecture="*" publicKeyToken="6595b64144ccf1df" type="win32" version="6.0.0.0"/>
		</dependentAssembly>
	</dependency>
</assembly>
"#,
	)
	.unwrap();
	let manifest = manifest.canonicalize().unwrap_or(manifest);
	println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
	println!("cargo:rustc-link-arg=/MANIFESTINPUT:{}", manifest.display());
	println!("cargo:rustc-link-arg=/MANIFESTUAC:NO");
}
