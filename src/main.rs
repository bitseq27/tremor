//! TREMOR entry point: spawns the network domain, then hands the primary
//! thread to the winit/wgpu render loop.

use std::sync::mpsc;

use tremor::app::{App, ShotCfg};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut shot: Option<ShotCfg> = None;
    let mut frames = 240u64;
    let mut path = String::from("tremor_snapshot.ppm");
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--frames" => {
                frames = args.get(i + 1).and_then(|v| v.parse().ok()).unwrap_or(240);
                i += 1;
            }
            "--shot" => {
                path = args.get(i + 1).cloned().unwrap_or(path);
                i += 1;
            }
            "--sim" => std::env::set_var("TREMOR_SIM", "1"),
            "--help" | "-h" => {
                println!(
                    "TREMOR - real-time 3D seismic monitor\n\
                     usage: tremor [--frames N --shot out.ppm] [--sim]\n\
                     --frames/--shot  render N frames headlessly then write a PPM snapshot\n\
                     --sim            force the offline simulation feed"
                );
                return;
            }
            other => eprintln!("tremor: ignoring unknown arg {other}"),
        }
        i += 1;
    }
    if args.iter().any(|a| a == "--frames" || a == "--shot") {
        shot = Some(ShotCfg {
            frames,
            path,
            done: false,
        });
    }

    let (tx, rx) = mpsc::sync_channel(64);
    let _net = tremor::net::spawn_feed(tx);

    let event_loop = winit::event_loop::EventLoop::builder()
        .build()
        .expect("event loop");
    let mut app = App::new(rx, shot);
    let _ = event_loop.run_app(&mut app);
}
