use std::error::Error;

fn main() -> Result<(), Box<dyn Error>> {
	env_logger::init();

	// ...existing code...

	Ok(())
}
