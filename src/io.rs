//! 画像ファイルの汎用 I/O (PNG 書き出し / フォルダからの画像収集)。

use std::path::Path;

/// Write a `width`×`height` RGB PNG. When `net` is given the RGB24 colours are
/// written as-is (the net-numbering representation already lives in u32); with
/// only `lit` the layer is exported binarized: ink (0) → black, paper (1) →
/// white, matching how the composite renders (lit==0 stays the black base).
pub fn write_image_png(
	path: &Path,
	width: u32,
	height: u32,
	net: Option<&[u32]>,
	lit: &[u8],
) -> Result<(), Box<dyn std::error::Error>> {
	let file = std::fs::File::create(path)?;
	let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), width, height);
	encoder.set_color(png::ColorType::Rgb);
	encoder.set_depth(png::BitDepth::Eight);
	let mut writer = encoder.write_header()?;
	let raw: Vec<u8> = if let Some(net) = net {
		net.iter()
			.flat_map(|&c| [(c >> 16) as u8, (c >> 8) as u8, c as u8])
			.collect()
	} else {
		lit.iter()
			.flat_map(|&b| {
				let v = if b > 0 { 255 } else { 0 };
				[v, v, v]
			})
			.collect()
	};
	writer.write_image_data(&raw)?;
	Ok(())
}

/// フォルダ内の画像ファイル (後で読める拡張子のみ) を再帰的に集める。
/// 順序が定まるようパス全体でソートして返す。
pub fn collect_image_files(dir: &Path, out: &mut Vec<std::path::PathBuf>) {
	let Ok(rd) = std::fs::read_dir(dir) else { return };
	for entry in rd.flatten() {
		let path = entry.path();
		if path.is_dir() {
			collect_image_files(&path, out);
		} else {
			let supported = path
				.extension()
				.and_then(|e| e.to_str())
				.map_or(false, |e| {
					matches!(e.to_ascii_lowercase().as_str(), "png" | "jpg" | "jpeg")
				});
			if supported {
				out.push(path);
			}
		}
	}
}