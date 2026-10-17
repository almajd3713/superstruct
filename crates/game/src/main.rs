use engine::window::{self, WindowConfig};

fn main() -> anyhow::Result<()> {
    window::run(WindowConfig {
        title: "Superstruct".into(),
        size: (1600, 900).into(),
    })?;
    Ok(())
}
