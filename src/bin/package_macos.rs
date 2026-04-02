#[allow(dead_code)]
#[path = "../app_icon.rs"]
mod app_icon;

#[cfg(target_os = "macos")]
mod macos {
    use std::error::Error;
    use std::fs;
    use std::fs::File;
    use std::io::BufWriter;
    use std::os::unix::fs::PermissionsExt;
    use std::path::{Path, PathBuf};
    use std::process::Command;

    use png::{BitDepth, ColorType, Encoder};

    const APP_NAME: &str = "Chess Engine";
    const BUNDLE_EXECUTABLE: &str = "chess_engine";
    const BUNDLE_IDENTIFIER: &str = "com.fred.chessengine";
    const ICON_FILE_NAME: &str = "ChessEngine.icns";
    const ICONSET_SPECS: [(&str, u32); 10] = [
        ("icon_16x16.png", 16),
        ("icon_16x16@2x.png", 32),
        ("icon_32x32.png", 32),
        ("icon_32x32@2x.png", 64),
        ("icon_128x128.png", 128),
        ("icon_128x128@2x.png", 256),
        ("icon_256x256.png", 256),
        ("icon_256x256@2x.png", 512),
        ("icon_512x512.png", 512),
        ("icon_512x512@2x.png", 1024),
    ];

    pub fn run() -> Result<(), Box<dyn Error>> {
        let repo_root = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        build_release_binary(&repo_root)?;

        let dist_dir = repo_root.join("dist").join("macos");
        let bundle_dir = dist_dir.join(format!("{APP_NAME}.app"));
        let contents_dir = bundle_dir.join("Contents");
        let macos_dir = contents_dir.join("MacOS");
        let resources_dir = contents_dir.join("Resources");
        let iconset_dir = dist_dir.join("ChessEngine.iconset");

        recreate_dir(&bundle_dir)?;
        recreate_dir(&iconset_dir)?;
        fs::create_dir_all(&macos_dir)?;
        fs::create_dir_all(&resources_dir)?;

        generate_iconset(&dist_dir, &iconset_dir)?;
        build_icns(&iconset_dir, &resources_dir.join(ICON_FILE_NAME))?;
        write_info_plist(&contents_dir.join("Info.plist"))?;
        copy_executable(&repo_root, &macos_dir.join(BUNDLE_EXECUTABLE))?;

        println!("Packaged app bundle at {}", bundle_dir.display());
        println!("Launch it with: open \"{}\"", bundle_dir.display());
        Ok(())
    }

    fn build_release_binary(repo_root: &Path) -> Result<(), Box<dyn Error>> {
        run_command(
            repo_root,
            "cargo",
            ["build", "--release", "--bin", BUNDLE_EXECUTABLE],
        )
    }

    fn generate_iconset(dist_dir: &Path, iconset_dir: &Path) -> Result<(), Box<dyn Error>> {
        let source_icon_path = dist_dir.join("ChessEngine-source.png");
        let source_icon = crate::app_icon::render_icon(1024);
        write_png(
            &source_icon_path,
            source_icon.width,
            source_icon.height,
            &source_icon.rgba,
        )?;

        for (file_name, size) in ICONSET_SPECS {
            let output_path = iconset_dir.join(file_name);
            run_command(
                dist_dir,
                "sips",
                [
                    "-z",
                    &size.to_string(),
                    &size.to_string(),
                    source_icon_path
                        .to_str()
                        .ok_or("source icon path should be utf-8")?,
                    "--out",
                    output_path
                        .to_str()
                        .ok_or("icon output path should be utf-8")?,
                ],
            )?;
        }

        fs::remove_file(source_icon_path)?;
        Ok(())
    }

    fn build_icns(iconset_dir: &Path, icns_path: &Path) -> Result<(), Box<dyn Error>> {
        if icns_path.exists() {
            fs::remove_file(icns_path)?;
        }

        let chunks = [
            ("icp4", "icon_16x16.png"),
            ("ic11", "icon_16x16@2x.png"),
            ("icp5", "icon_32x32.png"),
            ("ic12", "icon_32x32@2x.png"),
            ("ic07", "icon_128x128.png"),
            ("ic13", "icon_128x128@2x.png"),
            ("ic08", "icon_256x256.png"),
            ("ic14", "icon_256x256@2x.png"),
            ("ic09", "icon_512x512.png"),
            ("ic10", "icon_512x512@2x.png"),
        ];

        let mut total_length = 8_u32;
        let mut elements = Vec::with_capacity(chunks.len());
        for (chunk_type, file_name) in chunks {
            let data = fs::read(iconset_dir.join(file_name))?;
            let element_length = (8 + data.len()) as u32;
            total_length += element_length;
            elements.push((chunk_type, data));
        }

        let mut icns = Vec::with_capacity(total_length as usize);
        icns.extend_from_slice(b"icns");
        icns.extend_from_slice(&total_length.to_be_bytes());
        for (chunk_type, data) in elements {
            icns.extend_from_slice(chunk_type.as_bytes());
            icns.extend_from_slice(&((8 + data.len()) as u32).to_be_bytes());
            icns.extend_from_slice(&data);
        }

        fs::write(icns_path, icns)?;
        Ok(())
    }

    fn write_info_plist(plist_path: &Path) -> Result<(), Box<dyn Error>> {
        let plist = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleDevelopmentRegion</key>
    <string>en</string>
    <key>CFBundleDisplayName</key>
    <string>{APP_NAME}</string>
    <key>CFBundleExecutable</key>
    <string>{BUNDLE_EXECUTABLE}</string>
    <key>CFBundleIconFile</key>
    <string>{ICON_FILE_NAME}</string>
    <key>CFBundleIdentifier</key>
    <string>{BUNDLE_IDENTIFIER}</string>
    <key>CFBundleInfoDictionaryVersion</key>
    <string>6.0</string>
    <key>CFBundleName</key>
    <string>{APP_NAME}</string>
    <key>CFBundlePackageType</key>
    <string>APPL</string>
    <key>CFBundleShortVersionString</key>
    <string>{}</string>
    <key>CFBundleVersion</key>
    <string>{}</string>
    <key>LSApplicationCategoryType</key>
    <string>public.app-category.games</string>
    <key>NSHighResolutionCapable</key>
    <true/>
</dict>
</plist>
"#,
            env!("CARGO_PKG_VERSION"),
            env!("CARGO_PKG_VERSION"),
        );

        fs::write(plist_path, plist)?;
        Ok(())
    }

    fn copy_executable(repo_root: &Path, destination: &Path) -> Result<(), Box<dyn Error>> {
        let source = repo_root
            .join("target")
            .join("release")
            .join(BUNDLE_EXECUTABLE);
        fs::copy(&source, destination)?;

        let mut permissions = fs::metadata(destination)?.permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(destination, permissions)?;
        Ok(())
    }

    fn recreate_dir(path: &Path) -> Result<(), Box<dyn Error>> {
        if path.exists() {
            fs::remove_dir_all(path)?;
        }
        fs::create_dir_all(path)?;
        Ok(())
    }

    fn write_png(path: &Path, width: u32, height: u32, rgba: &[u8]) -> Result<(), Box<dyn Error>> {
        let file = File::create(path)?;
        let writer = BufWriter::new(file);
        let mut encoder = Encoder::new(writer, width, height);
        encoder.set_color(ColorType::Rgba);
        encoder.set_depth(BitDepth::Eight);

        let mut writer = encoder.write_header()?;
        writer.write_image_data(rgba)?;
        writer.finish()?;
        Ok(())
    }

    fn run_command<const N: usize>(
        cwd: &Path,
        program: &str,
        args: [&str; N],
    ) -> Result<(), Box<dyn Error>> {
        let status = Command::new(program).current_dir(cwd).args(args).status()?;
        if !status.success() {
            return Err(format!(
                "command failed: {} {}",
                program,
                args.join(" ")
            )
            .into());
        }
        Ok(())
    }
}

#[cfg(target_os = "macos")]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    macos::run()
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("`package_macos` is only available on macOS.");
    std::process::exit(1);
}
