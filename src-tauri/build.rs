use std::env;
use std::path::Path;
use std::process::Command;

const MANIFEST: &str = r#"<?xml version="1.0" encoding="UTF-8" standalone="yes"?>
<assembly xmlns="urn:schemas-microsoft-com:asm.v1" manifestVersion="1.0">
  <dependency>
    <dependentAssembly>
      <assemblyIdentity type="win32" name="Microsoft.Windows.Common-Controls" version="6.0.0.0" publicKeyToken="6595b64144ccf1df" language="*" processorArchitecture="*"/>
    </dependentAssembly>
  </dependency>
</assembly>
"#;

fn main() {
    // Windows: 测试可执行文件（cargo test）静态导入了 comctl32 v6 独有的函数
    // （TaskDialogIndirect / SetWindowSubclass 等，经 tauri/muda/rfd 引入），但 cargo
    // 不会为测试 exe 嵌入应用程序清单，导致加载器绑定到 System32 的 comctl32 v5，
    // 报 STATUS_ENTRYPOINT_NOT_FOUND。
    //
    // tauri-build 自带的清单通过 `rustc-link-arg-bins` 只作用于 bin 目标（主程序 +
    // bin 单元测试），lib 单元测试和集成测试无人覆盖；而 `rustc-link-arg-tests`
    // 实测只覆盖 tests/ 下的集成测试。cargo 没有任何指令能"只"覆盖 lib 单元测试。
    //
    // 因此统一方案：把清单资源编译进静态库，用
    //   cargo:rustc-link-lib=static:+whole-archive=test_manifest
    // 传播——link-lib 会随 lib 元数据传到**所有**最终链接产物（主程序、lib 单元
    // 测试、bin 单元测试、集成测试），+whole-archive 保证无符号的资源对象也会被
    // 提取（实测普通 -l 不会提取）。相应地用 new_without_app_manifest() 关闭
    // tauri-build 的默认清单，避免主程序出现两份 RT_MANIFEST ID 1（两者内容本就
    // 相同，仅 Common-Controls 依赖；图标与版本信息仍由 tauri 的 winres 正常嵌入）。
    if env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        let out_dir = env::var("OUT_DIR").expect("OUT_DIR not set");
        let manifest_path = Path::new(&out_dir).join("test_manifest.manifest");
        std::fs::write(&manifest_path, MANIFEST).expect("failed to write test manifest");

        match env::var("CARGO_CFG_TARGET_ENV").as_deref() {
            // MSVC: link.exe 的 /MANIFEST:EMBED 在每个链接命令上直接内嵌清单。
            // 用不带目标过滤的 rustc-link-arg 以覆盖单元测试等全部产物。
            Ok("msvc") => {
                println!("cargo:rustc-link-arg=/MANIFEST:EMBED");
                println!(
                    "cargo:rustc-link-arg=/MANIFESTINPUT:{}",
                    manifest_path.display()
                );
            }
            // GNU (MinGW): windres 把清单编译成 COFF 对象，ar 打包成静态库后用
            // +whole-archive 链入所有产物。
            _ => {
                let rc_path = Path::new(&out_dir).join("test_manifest.rc");
                let obj_path = Path::new(&out_dir).join("test_manifest.o");
                let archive_path = Path::new(&out_dir).join("libtest_manifest.a");
                // RT_MANIFEST (24) 资源，ID 1；windres 字符串中反斜杠是转义符，路径用正斜杠
                let manifest_fwd = manifest_path.display().to_string().replace('\\', "/");
                std::fs::write(&rc_path, format!("1 24 \"{}\"", manifest_fwd))
                    .expect("failed to write test manifest rc");
                compile_manifest_resource(&rc_path, &obj_path)
                    .expect("windres not found to embed test manifest");
                build_archive(&obj_path, &archive_path)
                    .expect("ar not found to bundle test manifest");
                println!("cargo:rustc-link-search=native={}", out_dir);
                println!("cargo:rustc-link-lib=static:+whole-archive=test_manifest");
            }
        }
    }

    let attrs =
        tauri_build::Attributes::new().windows_attributes(tauri_build::WindowsAttributes::new_without_app_manifest());
    tauri_build::try_build(attrs).expect("failed to run tauri-build");
}

fn compile_manifest_resource(rc: &Path, obj: &Path) -> Option<std::path::PathBuf> {
    for tool in ["x86_64-w64-mingw32-windres", "windres"] {
        let out = Command::new(tool)
            .args(["-O", "coff"])
            .arg(rc)
            .arg(obj)
            .output();
        match out {
            Ok(o) if o.status.success() && obj.exists() => return Some(obj.to_path_buf()),
            Ok(o) => {
                println!(
                    "cargo:warning=windres '{}' failed ({}): {}",
                    tool,
                    o.status,
                    String::from_utf8_lossy(&o.stderr).trim()
                );
            }
            Err(e) => {
                println!("cargo:warning=windres '{}' not runnable: {}", tool, e);
            }
        }
    }
    None
}

fn build_archive(obj: &Path, archive: &Path) -> Option<()> {
    // 幂等：清掉旧档案再打包，避免陈旧成员残留
    let _ = std::fs::remove_file(archive);
    for tool in ["ar", "x86_64-w64-mingw32-ar"] {
        let out = Command::new(tool)
            .args(["crs"])
            .arg(archive)
            .arg(obj)
            .output();
        match out {
            Ok(o) if o.status.success() && archive.exists() => return Some(()),
            Ok(o) => {
                println!(
                    "cargo:warning=ar '{}' failed ({}): {}",
                    tool,
                    o.status,
                    String::from_utf8_lossy(&o.stderr).trim()
                );
            }
            Err(e) => {
                println!("cargo:warning=ar '{}' not runnable: {}", tool, e);
            }
        }
    }
    None
}
