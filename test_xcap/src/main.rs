use xcap::Monitor;

fn main() {
    let monitors = Monitor::all().unwrap();
    let monitor = &monitors[0];
    let image = monitor.capture_image().unwrap();
    image.save("/tmp/test_xcap.png").unwrap();
    println!("Screenshot saved. Size: {}x{}", image.width(), image.height());
}
