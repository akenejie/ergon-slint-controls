fn main() {
	slint_build::compile_with_config(
		"ui/lib.slint",
		slint_build::CompilerConfiguration::new().as_library("ergon-slint-controls"),
	)
	.unwrap();
}