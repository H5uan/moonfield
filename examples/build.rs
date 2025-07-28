use std::{
    env,
    fs,
    io::Result,
    path::{Path, PathBuf},
    process::{Command, Output},
};

fn main() {
    if !should_skip_shader_compilation() {
        compile_shaders();
    }
}

fn should_skip_shader_compilation() -> bool {
    env::var("SKIP_SHADER_COMPILATION")
        .map(|var| var.parse::<bool>().unwrap_or(false))
        .unwrap_or(false)
}

fn compile_shaders() {
    println!("Compiling shaders");

    let slangc_path = find_slangc_compiler();
    if slangc_path.as_os_str().is_empty() {
        println!("cargo:warning=Slang编译器未找到，跳过着色器编译");
        return;
    }

    let shader_dir_path = get_shader_source_dir_path();

    fs::read_dir(shader_dir_path.clone())
        .unwrap()
        .map(Result::unwrap)
        .filter(|dir| dir.file_type().unwrap().is_file())
        .filter(|dir| {
            let path = dir.path();
            let extension = path.extension().and_then(|ext| ext.to_str());
            // 编译着色器文件，排除已编译的SPV文件
            extension != Some("spv") && extension == Some("slang")
        })
        .for_each(|dir| {
            let path = dir.path();
            let name = path.file_name().unwrap().to_str().unwrap();
            let output_name = format!("{}.spv", &name);
            
            println!("Compiling shader: {:?}", path.as_os_str());

            // 根据文件名确定着色器类型
            let shader_stage = if name.contains("vert") {
                "vertex"  // 顶点着色器
            } else if name.contains("frag") {
                "fragment"  // 像素着色器
            } else if name.contains("comp") {
                "compute"  // 计算着色器
            } else if name.contains("geom") {
                "geometry"  // 几何着色器
            } else if name.contains("hull") {
                "hull"  // 外壳着色器
            } else if name.contains("domain") {
                "domain"  // 域着色器
            } else if name.contains("mesh") {
                "mesh"  // 网格着色器
            } else if name.contains("amplification") {
                "amplification"  // 放大着色器
            } else {
                // 默认为像素着色器
                println!("Warning: Could not determine shader type for {:?}, defaulting to fragment shader", name);
                "fragment"
            };

            // 构建编译命令
            let result = Command::new(&slangc_path)
                .current_dir(&shader_dir_path)
                .arg("-target")
                .arg("spirv")  // 输出SPIR-V格式
                .arg("-stage")
                .arg(shader_stage)
                .arg("-entry")
                .arg("main")  // 入口点
                .arg("-o")
                .arg(&output_name)
                .arg(&path)
                .output();
            
            handle_program_result(result);
        })
}

fn get_shader_source_dir_path() -> PathBuf {
    // 由于build.rs现在在examples目录中，需要向上一级目录查找assets
    let path = get_root_path().parent().unwrap().join("assets").join("shaders");
    println!("Shader source directory: {:?}", path.as_os_str());
    path
}

fn find_slangc_compiler() -> PathBuf {
    // 首先检查是否有环境变量指定SLANGC路径
    if let Ok(slangc_path) = env::var("SLANGC_PATH") {
        let path = PathBuf::from(slangc_path);
        if path.exists() {
            return path;
        }
    }

    // 尝试从Vulkan SDK环境变量获取路径
    if let Ok(vulkan_sdk) = env::var("VULKAN_SDK") {
        let path = PathBuf::from(vulkan_sdk).join("Bin").join("slangc.exe");
        if path.exists() {
            return path;
        }
    }

    // 尝试在系统PATH中查找slangc
    if let Ok(output) = Command::new("which").arg("slangc").output() {
        if output.status.success() {
            if let Ok(path_str) = String::from_utf8(output.stdout) {
                let path_str = path_str.trim();
                if !path_str.is_empty() {
                    let path = PathBuf::from(path_str);
                    if path.exists() {
                        return path;
                    }
                }
            }
        }
    }

    println!("cargo:warning=无法找到Slang编译器。着色器编译将被跳过。请安装Slang Shader Compiler或Vulkan SDK，或设置SLANGC_PATH环境变量指向slangc可执行文件。");
    return PathBuf::new();
}

fn get_root_path() -> &'static Path {
    Path::new(env!("CARGO_MANIFEST_DIR"))
}

fn handle_program_result(result: Result<Output>) {
    match result {
        Ok(output) => {
            if output.status.success() {
                println!("Shader compilation succeeded.");
                print!(
                    "stdout: {}",
                    String::from_utf8(output.stdout)
                        .unwrap_or("Failed to print program stdout".to_string())
                );
            } else {
                eprintln!("Shader compilation failed. Status: {}", output.status);
                eprint!(
                    "stdout: {}",
                    String::from_utf8(output.stdout)
                        .unwrap_or("Failed to print program stdout".to_string())
                );
                eprint!(
                    "stderr: {}",
                    String::from_utf8(output.stderr)
                        .unwrap_or("Failed to print program stderr".to_string())
                );
                panic!("Shader compilation failed. Status: {}", output.status);
            }
        }
        Err(error) => {
            panic!("Failed to compile shader. Cause: {}", error);
        }
    }
}
