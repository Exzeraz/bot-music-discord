use chrono::Local;
use colored::Colorize;

fn ts() -> String {
    Local::now().format("%Y-%m-%d %H:%M:%S").to_string()
}

pub fn info(msg: impl AsRef<str>) {
    println!("{} {}", format!("[{}][INFO]", ts()).cyan(), msg.as_ref());
}

pub fn success(msg: impl AsRef<str>) {
    println!("{} {}", format!("[{}][SUCCESS]", ts()).green(), msg.as_ref());
}

pub fn error(msg: impl AsRef<str>) {
    println!("{} {}", format!("[{}][ERROR]", ts()).red().bold(), msg.as_ref());
}

pub fn command(user: &str, name: &str, details: &str) {
    println!(
        "{} {} executed {} -> {}",
        format!("[{}][CMD]", ts()).magenta(),
        user.yellow().bold(),
        format!("!{name}").cyan().bold(),
        details
    );
}
