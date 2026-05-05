use std::{env, fs::File};

use crate::emulator::Machine;

use anyhow::{Ok, Result, bail};

mod emulator;

fn main() -> Result<()> {
    let args: Vec<String> = env::args().collect();

    let path = match args.get(1) {
        Some(x) => x,
        None => {
            println!("You didn't give a rom. Pass the path as an argument. The second argument is the cycles per frame (Speed. Defaults to 1).\nchip8 path/to/rom 10 # run a rom with 10 cycles per frame");
            bail!("Bad args");
        }
    };
    let file = File::open(path)?;

    let cycles = match args.get(2) {
        Some(x) => x.parse()?,
        None => 1,
    };

    let mut machine = Machine::new()?;

    machine.interpret(file, cycles)?;

    Ok(())
}
