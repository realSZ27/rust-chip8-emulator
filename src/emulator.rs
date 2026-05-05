use std::{fs::File, io::Read, time::{Duration, Instant}};

use minifb::{Key, Window, WindowOptions};
use rand::random_range;
use rodio::{OutputStream, OutputStreamBuilder, Sink, Source, source::SineWave};

use anyhow::{Ok, Result, bail};

pub struct Machine {
    cpu: Cpu,
    last_timer_tick: Instant,
    window: Window, // handles keypresses and display
    buffer: Vec<u32>, // diplay buffer
    audio_stream: OutputStream,
    sound_sink: Option<Sink>,
}

pub struct Cpu {
    memory: [u8; 4096],
    stack: Vec<u16>,
    gpio: [u8; 16],
    sound_timer: u8,
    delay_timer: u8,
    index: u16,
    pc: u16,
}

impl Machine {
    pub fn new() -> Result<Machine> {
        let mut options = WindowOptions::default();
        options.scale = minifb::Scale::X16;

        let window = Window::new(
            "Chip-8 Emulator", 
            64, 
            32, 
            options
        )?;
        let sound_stream = OutputStreamBuilder::open_default_stream()?;

        Ok(Machine { 
            cpu: Cpu { 
                memory: [0; 4096], 
                stack: Vec::new(),
                gpio: [0; 16], 
                sound_timer: 0, 
                delay_timer: 0, 
                index: 0, 
                pc: 0x200 
            }, 
            last_timer_tick: Instant::now(),
            window: window,
            buffer: vec![0; 64 * 32],
            audio_stream: sound_stream,
            sound_sink: None,
        })
    }

    pub fn interpret(&mut self, rom: File, cycles: usize) -> Result<()> {      
        self.write_font();

        self.load_rom(rom)?;

        while self.window.is_open() {
            for _ in 0..cycles {
                self.cycle()?;
            }

            self.window.update_with_buffer(&self.buffer, 64, 32)?;
        }

        Ok(())
    }

    fn write_font(&mut self) {
        let font = [
                0xF0, 0x90, 0x90, 0x90, 0xF0, // 0
                0x20, 0x60, 0x20, 0x20, 0x70, // 1
                0xF0, 0x10, 0xF0, 0x80, 0xF0, // 2
                0xF0, 0x10, 0xF0, 0x10, 0xF0, // 3
                0x90, 0x90, 0xF0, 0x10, 0x10, // 4
                0xF0, 0x80, 0xF0, 0x10, 0xF0, // 5
                0xF0, 0x80, 0xF0, 0x90, 0xF0, // 6
                0xF0, 0x10, 0x20, 0x40, 0x40, // 7
                0xF0, 0x90, 0xF0, 0x90, 0xF0, // 8
                0xF0, 0x90, 0xF0, 0x10, 0xF0, // 9
                0xF0, 0x90, 0xF0, 0x90, 0x90, // A
                0xE0, 0x90, 0xE0, 0x90, 0xE0, // B
                0xF0, 0x80, 0x80, 0x80, 0xF0, // C
                0xE0, 0x90, 0x90, 0x90, 0xE0, // D
                0xF0, 0x80, 0xF0, 0x80, 0xF0, // E
                0xF0, 0x80, 0xF0, 0x80, 0x80  // F
            ];
        
        self.cpu.memory[0..(0 + font.len())].copy_from_slice(&font);
    }

    fn load_rom(&mut self, mut rom: File) -> Result<()> {
        let meta = rom.metadata()?;
        println!("ROM metadata size: {} bytes", meta.len());

        let mut buffer = Vec::new();
        rom.read_to_end(&mut buffer)?;
        println!("Bytes actually read: {}", buffer.len());

        let max_size = 4096 - 0x200;
        if buffer.len() > max_size {
            bail!(
                "ROM too large: {} bytes (max is {})",
                buffer.len(),
                max_size
            );
        }

        self.cpu.memory[0x200..0x200 + buffer.len()]
            .copy_from_slice(&buffer);

        Ok(())
    }

    fn cycle(&mut self) -> Result<()> {
        let opcode = (self.cpu.memory[self.cpu.pc as usize] as u16) << 8 
                   | (self.cpu.memory[self.cpu.pc as usize + 1] as u16);

        self.cpu.pc += 2;

        self.handle_opcode(opcode)?;

        let now = Instant::now();
        if now.duration_since(self.last_timer_tick) >= Duration::from_secs_f64(1.0 / 60.0) {
            if self.cpu.delay_timer > 0 {
                self.cpu.delay_timer -= 1;
            }
            if self.cpu.sound_timer > 0 {
                self.cpu.sound_timer -= 1;
            }
            self.last_timer_tick = now;
        }

        if self.cpu.sound_timer > 0 {
            if self.sound_sink.is_none() {
                let sink = Sink::connect_new(&self.audio_stream.mixer());
            
                let tone = SineWave::new(440.0)
                    .amplify(0.20)
                    .repeat_infinite();
            
                sink.append(tone);
                sink.play();
            
                self.sound_sink = Some(sink);
            }
        } else {
            // Stop sound when timer reaches zero
            if let Some(sink) = self.sound_sink.take() {
                sink.stop();
            }
        }

        Ok(())
    }

    fn handle_opcode(&mut self, opcode: u16) -> Result<()> {
        match opcode {
            0x00E0 => self.clear_screen(),
            0x00EE => self.return_subroutine(),
            0x1000..=0x1FFF => self.jump(opcode),
            0x2000..=0x2FFF => self.push_addr_and_call_sub(opcode),
            0x3000..=0x3FFF => self.skip_if_reg_equal(opcode),
            0x4000..=0x4FFF => self.skip_if_reg_ne(opcode),
            0x5000..=0x5FFF => self.skip_if_reg_equal_reg(opcode),
            0x6000..=0x6FFF => self.set_reg(opcode),
            0x7000..=0x7FFF => self.add_val_to_reg(opcode),
            0x8000..=0x8FFF => self.handle_8_opcodes(opcode)?,
            0x9000..=0x9FFF => self.skip_if_ne_reg_reg(opcode),
            0xA000..=0xAFFF => self.set_index_reg(opcode),
            0xB000..=0xBFFF => self.jump_add_reg(opcode),
            0xC000..=0xCFFF => self.random(opcode),
            0xD000..=0xDFFF => self.draw_sprite(opcode),
            0xE000..=0xEFFF => {
                match opcode & 0x00FF {
                    0x9E => self.is_key_pressed(opcode, true)?,
                    0xA1 => self.is_key_pressed(opcode, false)?,
                    _ => bail!("Unknown opcode {:04X}", opcode)
                }
            }
            0xF000..=0xFFFF => self.handle_f_timers(opcode)?,
            _ => bail!("Unknown opcode {:04X}", opcode)
        }

        Ok(())
    }

    fn clear_screen(&mut self) {
        self.buffer.fill(0);
    }

    fn return_subroutine(&mut self) {
        self.cpu.pc = self.cpu.stack.pop().unwrap();
    }

    fn jump(&mut self, opcode: u16) {
        self.cpu.pc = opcode & 0x0FFF;
    }

    fn push_addr_and_call_sub(&mut self, opcode: u16) {
        self.cpu.stack.push(self.cpu.pc);
        self.cpu.pc = opcode & 0x0FFF;
    }

    fn set_reg(&mut self, opcode: u16) {
        let register = (opcode & 0x0F00) >> 8;
        let value = (opcode & 0x00FF) as u8;

        self.cpu.gpio[register as usize] = value;
    }

    fn add_val_to_reg(&mut self, opcode: u16) {
        let register = (opcode & 0x0F00) >> 8;
        let value = (opcode & 0x00FF) as u8;

        self.cpu.gpio[register as usize] = self.cpu.gpio[register as usize].wrapping_add(value);
    }

    fn handle_8_opcodes(&mut self, opcode: u16) -> Result<()> {
        let x = ((opcode & 0x0F00) >> 8) as usize;
        let y = ((opcode & 0x00F0) >> 4) as usize;

        let regx = self.cpu.gpio[x];
        let regy = self.cpu.gpio[y];

        match opcode & 0x000F {
            0x0 => {
                self.cpu.gpio[x] = regy;
            },
            0x1 => {
                self.cpu.gpio[x] = regx | regy;
                self.cpu.gpio[0xF] = 0;
            },
            0x2 => {
                self.cpu.gpio[x] = regx & regy;
                self.cpu.gpio[0xF] = 0;
            },
            0x3 => {
                self.cpu.gpio[x] = regx ^ regy;
                self.cpu.gpio[0xF] = 0;
            },
            0x4 => {
                let result = regx as usize + regy as usize;

                self.cpu.gpio[x] = result as u8;
                self.cpu.gpio[0xF] = if result > 255 { 1 } else { 0 };
            },
            0x5 => {
                self.cpu.gpio[x] = regx.wrapping_sub(regy);
                self.cpu.gpio[0xF] = if regx >= regy { 1 } else { 0 };
            },
            0x6 => {
                self.cpu.gpio[x] = regy >> 1;
                self.cpu.gpio[0xF] = regy & 0x01;
            },
            0x7 => {
                self.cpu.gpio[x] = regy.wrapping_sub(regx);
                self.cpu.gpio[0xF] = if regy >= regx { 1 } else { 0 };
            },
            0xE => {
                self.cpu.gpio[x] = regy << 1;
                self.cpu.gpio[0xF] = (regy >> 7) & 1;
            },
            _ => { bail!("Unknown opcode {:04X}", opcode); }
        }

        Ok(())
    }

    fn skip_if_ne_reg_reg(&mut self, opcode: u16) {
        let x = ((opcode & 0x0F00) >> 8) as usize;
        let y = ((opcode & 0x00F0) >> 4) as usize;

        if self.cpu.gpio[x] != self.cpu.gpio[y] {
            self.cpu.pc += 2;
        }
    }

    fn skip_if_reg_equal(&mut self, opcode: u16) {
        let register = (opcode & 0x0F00) >> 8;
        let value = (opcode & 0x00FF) as u8;

        if self.cpu.gpio[register as usize] == value {
            self.cpu.pc += 2;
        }   
    }

    fn skip_if_reg_ne(&mut self, opcode: u16) {
        let register = (opcode & 0x0F00) >> 8;
        let value = (opcode & 0x00FF) as u8;

        if self.cpu.gpio[register as usize] != value {
            self.cpu.pc += 2;
        }
    }

    fn skip_if_reg_equal_reg(&mut self, opcode: u16) {
        let registerx = (opcode & 0x0F00) >> 8;
        let registery = (opcode & 0x00F0) >> 4;

        if self.cpu.gpio[registerx as usize] == self.cpu.gpio[registery as usize] {
            self.cpu.pc += 2;
        }
    }

    fn set_index_reg(&mut self, opcode: u16) {
        let value = opcode & 0x0FFF;

        self.cpu.index = value;
    }

    fn jump_add_reg(&mut self, opcode: u16) {
        let value = opcode & 0x0FFF;

        self.cpu.pc = value + self.cpu.gpio[0] as u16;
    }

    fn random(&mut self, opcode: u16) {
        let register = (opcode & 0x0F00) >> 8;
        let value = (opcode & 0x00FF) as u8;

        self.cpu.gpio[register as usize] = random_range(0..=255) & value;
    }

    fn draw_sprite(&mut self, opcode: u16) {
        let regx = self.cpu.gpio[((opcode & 0x0F00) >> 8) as usize];
        let regy = self.cpu.gpio[((opcode & 0x00F0) >> 4) as usize];
        let n = (opcode & 0x000F) as usize;

        self.cpu.gpio[0xF] = 0;

        for row in 0..n {
            let sprite_byte = self.cpu.memory[self.cpu.index as usize + row];

            for col in 0..8 {
                let sprite_pixel = (sprite_byte >> (7 - col)) & 1;

                if sprite_pixel == 0 {
                    continue;
                }

                let px = (regx as usize + col) % 64;
                let py = (regy as usize + row) % 32;
                let idx = py as usize * 64 + px as usize;

                if self.buffer[idx as usize] != 0 {
                    self.cpu.gpio[0xF] = 1;
                }

                self.buffer[idx as usize] ^= 0xFFFFFFFF;
            }
        }
    }

    fn is_key_pressed(&mut self, opcode: u16, not: bool) -> Result<()> {
        let register = (opcode & 0x0F00) >> 8;

        let key = match self.cpu.gpio[register as usize] {
            0x1 => Key::Key1, 0x2 => Key::Key2, 0x3 => Key::Key3, 0xC => Key::Key4,
            0x4 => Key::Q, 0x5 => Key::W, 0x6 => Key::E, 0xD => Key::R,
            0x7 => Key::A, 0x8 => Key::S, 0x9 => Key::D, 0xE => Key::F,
            0xA => Key::Z, 0x0 => Key::X, 0xB => Key::C, 0xF => Key::V,
            _ => {
                eprintln!("opcode {:04X}: The value of V{} is not a valid key ({})", opcode, register, self.cpu.gpio[register as usize]);
                return Ok(())
            },
        };

        if not {
            if self.window.is_key_down(key) {
                self.cpu.pc += 2;
            }
        } else {
            if !self.window.is_key_down(key) {
                self.cpu.pc += 2;
            }
        }

        Ok(())
    }

    fn handle_f_timers(&mut self, opcode: u16) -> Result<()> {
        let rhs = opcode & 0x00FF;
        let register = (opcode & 0x0F00) >> 8;

        match rhs {
            0x07 => {
                self.cpu.gpio[register as usize] = self.cpu.delay_timer;
            },
            0x0A => {
                let keys = self.window.get_keys_pressed(minifb::KeyRepeat::No);

                if keys.is_empty() {
                    self.cpu.pc -= 2;
                    return Ok(());
                }

                let hex_value = match keys[0] {
                    Key::Key1 => 0x1, Key::Key2 => 0x2, Key::Key3 => 0x3, Key::Key4 => 0x4,
                    Key::Q => 0x5, Key::W => 0x6, Key::E => 0x7, Key::R => 0x8,
                    Key::A => 0x9, Key::S => 0xA, Key::D => 0xB, Key::F => 0xC,
                    Key::Z => 0xD, Key::X => 0xE, Key::C => 0xF,
                    _ => bail!("Invalid key {:?}", keys[0]),
                };

                self.cpu.gpio[register as usize] = hex_value;
            },
            0x15 => {
                self.cpu.delay_timer = self.cpu.gpio[register as usize];
            },
            0x18 => {
                self.cpu.sound_timer = self.cpu.gpio[register as usize];
            },
            0x1E => {
                self.cpu.index += self.cpu.gpio[register as usize] as u16;
            },
            0x29 => {
                let value = self.cpu.gpio[register as usize];
                self.cpu.index = (value * 5) as u16;
            },
            0x33 => {
                let regx = self.cpu.gpio[register as usize];

                self.cpu.memory[(self.cpu.index) as usize] = (regx / 100) % 10;
                self.cpu.memory[(self.cpu.index + 1) as usize] = (regx / 10) % 10;
                self.cpu.memory[(self.cpu.index + 2) as usize] = regx % 10;
            },
            0x55 => {
                for i in 0..=register {
                    self.cpu.memory[(self.cpu.index + i) as usize] = self.cpu.gpio[i as usize];
                }
            },
            0x65 => {
                for i in 0..=register {
                    self.cpu.gpio[i as usize] = self.cpu.memory[(self.cpu.index + i) as usize];
                }
            },
            _ => bail!("Unknown opcode {:04X}", opcode)
        }

        Ok(())
    }
}