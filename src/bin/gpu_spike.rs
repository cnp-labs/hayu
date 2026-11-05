use std::io::Write;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Instant;

use hayu::gpu::{self, Backend, OffscreenOutput, Result};

struct Args {
    backend: Backend,
    present: bool,
    frames: u32,
    dump: Option<PathBuf>,
    json: bool,
    scale: u32,
    hold: f64,
}

fn main() -> ExitCode {
    let start = Instant::now();
    match run(start) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("hayu-gpu-spike: {err}");
            ExitCode::FAILURE
        }
    }
}

fn run(start: Instant) -> Result<ExitCode> {
    let args = parse_args()?;
    let scene = gpu::scene::Scene::build(args.scale)?;

    if args.present {
        let out = gpu::present::run(args.backend, args.frames, args.hold, scene, start)?;
        let p50 = percentile(&out.times_ms, 0.50);
        let p95 = percentile(&out.times_ms, 0.95);
        if args.json {
            println!(
                "{}",
                serde_json::json!({
                    "backend": args.backend.name(),
                    "mode": "present",
                    "frames": args.frames,
                    "startup_ms": round3(out.startup_ms),
                    "frame_p50_ms": round3(p50),
                    "frame_p95_ms": round3(p95),
                    "scale": args.scale,
                    "width": out.width,
                    "height": out.height,
                    "renderer": out.renderer,
                })
            );
        } else {
            println!(
                "backend={} mode=present frames={} startup_ms={:.3} p50_ms={:.3} p95_ms={:.3} renderer={}",
                args.backend.name(),
                args.frames,
                out.startup_ms,
                p50,
                p95,
                out.renderer
            );
            let frames: Vec<String> = out
                .times_ms
                .iter()
                .enumerate()
                .map(|(index, ms)| format!("{index}:{ms:.3}"))
                .collect();
            println!("frames_ms=[{}]", frames.join(" "));
        }
        return Ok(ExitCode::SUCCESS);
    }

    let frames = args.frames;
    let out = offscreen(args.backend, &scene, frames)?;
    if let Some(path) = &args.dump {
        write_ppm(path, out.width, out.height, &out.pixels)?;
    }
    let p50 = percentile(&out.times_ms, 0.50);
    let p95 = percentile(&out.times_ms, 0.95);
    if args.json {
        println!(
            "{}",
            serde_json::json!({
                "backend": args.backend.name(),
                "mode": "offscreen",
                "frames": frames,
                "startup_ms": serde_json::Value::Null,
                "frame_p50_ms": round3(p50),
                "frame_p95_ms": round3(p95),
                "scale": args.scale,
                "width": out.width,
                "height": out.height,
                "renderer": out.renderer,
                "dump": args.dump.as_ref().map(|p| p.display().to_string()),
            })
        );
    } else {
        println!(
            "backend={} mode=offscreen frames={} p50_ms={:.3} p95_ms={:.3} size={}x{} renderer={}",
            args.backend.name(),
            frames,
            p50,
            p95,
            out.width,
            out.height,
            out.renderer
        );
        if let Some(path) = &args.dump {
            println!("dump={}", path.display());
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn offscreen(backend: Backend, scene: &gpu::scene::Scene, frames: u32) -> Result<OffscreenOutput> {
    match backend {
        Backend::Cpu => {
            let out = gpu::cpu::render(scene, frames)?;
            Ok(OffscreenOutput {
                pixels: out.pixels,
                width: out.width,
                height: out.height,
                times_ms: out.times_ms,
                renderer: "tiny-skia 0.12".into(),
            })
        }
        Backend::Gl => {
            #[cfg(feature = "gl")]
            {
                gpu::gl::offscreen(scene, frames)
            }
            #[cfg(not(feature = "gl"))]
            {
                Err(gpu::Error::Usage(
                    "this binary was built without the gl feature".into(),
                ))
            }
        }
        Backend::Wgpu => {
            #[cfg(feature = "wgpu")]
            {
                gpu::wgpu::offscreen(scene, frames)
            }
            #[cfg(not(feature = "wgpu"))]
            {
                Err(gpu::Error::Usage(
                    "this binary was built without the wgpu feature".into(),
                ))
            }
        }
    }
}

fn parse_args() -> Result<Args> {
    let mut backend = Backend::Cpu;
    let mut present = false;
    let mut frames = None;
    let mut dump = None;
    let mut json = false;
    let mut scale = 1;
    let mut hold = 0.0;
    let mut args = std::env::args().skip(1);
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--backend" => {
                let value = args
                    .next()
                    .ok_or_else(|| gpu::Error::Usage("--backend needs a value".into()))?;
                backend = Backend::parse(&value).ok_or_else(|| {
                    gpu::Error::Usage(format!("unknown backend {value:?} (cpu|gl|wgpu)"))
                })?;
            }
            "--mode" => {
                let value = args
                    .next()
                    .ok_or_else(|| gpu::Error::Usage("--mode needs a value".into()))?;
                match value.as_str() {
                    "offscreen" => present = false,
                    "present" => present = true,
                    other => {
                        return Err(gpu::Error::Usage(format!(
                            "unknown mode {other:?} (offscreen|present)"
                        )));
                    }
                }
            }
            "--frames" => {
                let value = args
                    .next()
                    .ok_or_else(|| gpu::Error::Usage("--frames needs a value".into()))?;
                frames = Some(parse_u32(&value, "--frames")?);
            }
            "--dump" => {
                dump = Some(PathBuf::from(
                    args.next()
                        .ok_or_else(|| gpu::Error::Usage("--dump needs a path".into()))?,
                ));
            }
            "--json" => json = true,
            "--scale" => {
                let value = args
                    .next()
                    .ok_or_else(|| gpu::Error::Usage("--scale needs a value".into()))?;
                scale = parse_u32(&value, "--scale")?;
                gpu::scene::validate_scale(scale)?;
            }
            "--hold" => {
                let value = args
                    .next()
                    .ok_or_else(|| gpu::Error::Usage("--hold needs seconds".into()))?;
                hold = value.parse::<f64>().map_err(|_| {
                    gpu::Error::Usage(format!("--hold expects seconds, got {value:?}"))
                })?;
                if !(0.0..=3600.0).contains(&hold) {
                    return Err(gpu::Error::Usage("--hold must be 0..=3600 seconds".into()));
                }
            }
            "--help" | "-h" => {
                println!(
                    "hayu-gpu-spike --backend cpu|gl|wgpu --mode offscreen|present [--frames N] \
                     [--dump PATH.ppm] [--json] [--scale 1..4] [--hold SECONDS]"
                );
                std::process::exit(0);
            }
            other => {
                return Err(gpu::Error::Usage(format!("unknown argument {other:?}")));
            }
        }
    }
    let frames = frames.unwrap_or(if present { 120 } else { 300 });
    if frames == 0 {
        return Err(gpu::Error::Usage("--frames must be at least 1".into()));
    }
    Ok(Args {
        backend,
        present,
        frames,
        dump,
        json,
        scale,
        hold,
    })
}

fn parse_u32(value: &str, flag: &str) -> Result<u32> {
    value
        .parse::<u32>()
        .map_err(|_| gpu::Error::Usage(format!("{flag} expects an integer, got {value:?}")))
}

fn percentile(times: &[f64], fraction: f64) -> f64 {
    if times.is_empty() {
        return 0.0;
    }
    let mut sorted = times.to_vec();
    sorted.sort_by(|a, b| a.total_cmp(b));
    let index = ((sorted.len() - 1) as f64 * fraction).round() as usize;
    sorted[index]
}

fn round3(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

fn write_ppm(path: &std::path::Path, width: u32, height: u32, rgba: &[u8]) -> Result<()> {
    let mut file = std::fs::File::create(path)
        .map_err(|err| gpu::Error::Io(format!("{}: {err}", path.display())))?;
    write!(file, "P6\n{width} {height}\n255\n")
        .map_err(|err| gpu::Error::Io(format!("{}: {err}", path.display())))?;
    let mut rgb = Vec::with_capacity((width * height * 3) as usize);
    for pixel in rgba.chunks_exact(4) {
        rgb.extend_from_slice(&pixel[..3]);
    }
    file.write_all(&rgb)
        .map_err(|err| gpu::Error::Io(format!("{}: {err}", path.display())))?;
    Ok(())
}
