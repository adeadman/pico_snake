//! Blink the RP Pico W onboard LED
//! Does not work with the RP Pico (non-W) which uses GPIO pin 25 to drive the onboard LED

#![no_std]
#![no_main]
#![feature(type_alias_impl_trait)]

use core::cell::RefCell;

use defmt::info;
use embassy_embedded_hal::shared_bus::blocking::spi::SpiDeviceWithConfig;
use embassy_executor::Spawner;
use embassy_rp::gpio::{Input, Level, Output, Pull};
use embassy_rp::spi;
use embassy_rp::spi::{Blocking, Spi};
use embassy_rp::clocks::RoscRng;
use embassy_sync::blocking_mutex::raw::NoopRawMutex;
use embassy_sync::blocking_mutex::Mutex;
use embassy_time::{Delay, Duration, Timer};
use embedded_graphics::{
    mono_font::{ascii::FONT_10X20, MonoTextStyle},
    pixelcolor::Rgb565,
    prelude::*,
    primitives::{Rectangle, PrimitiveStyleBuilder},
    text::Text,
};
use mipidsi::Builder;
use rand::RngCore;
use {defmt_rtt as _, panic_probe as _};

use heapless::spsc::Queue;
use arrayvec::ArrayString;
use core::fmt::Write;

pub mod display;
use display::SPIDeviceInterface;

const H: i32 = 240;
const W: i32 = 240;

const DISPLAY_FREQ: u32 = 64_000_000;


#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let p = embassy_rp::init(Default::default());
    info!("Hello RP2040!");

    let mut rng = RoscRng;

    let bl = p.PIN_13;
    let rst = p.PIN_12;
    let display_cs = p.PIN_9;
    let dcx = p.PIN_8;
    let mosi = p.PIN_11;
    // miso is not needed as there is only a master-to-slave data output for display
    let clk = p.PIN_10;

    let btn_a = Input::new(p.PIN_15, Pull::Up);
    let _btn_b = Input::new(p.PIN_17, Pull::Up);
    let _btn_x = Input::new(p.PIN_19, Pull::Up);
    let btn_y = Input::new(p.PIN_21, Pull::Up);

    let btn_u = Input::new(p.PIN_2, Pull::Up);
    let btn_d = Input::new(p.PIN_18, Pull::Up);
    let btn_l = Input::new(p.PIN_16, Pull::Up);
    let btn_r = Input::new(p.PIN_20, Pull::Up);
    let _btn_c = Input::new(p.PIN_3, Pull::Up);

    // Set up Serial Peripheral Interface (SPI)
    let mut display_config = spi::Config::default();
    display_config.frequency = DISPLAY_FREQ;
    display_config.phase = spi::Phase::CaptureOnSecondTransition;
    display_config.polarity = spi::Polarity::IdleHigh;

    let spi: Spi<'_, _, Blocking> = Spi::new_blocking_txonly(p.SPI1, clk, mosi, display_config.clone());
    let spi_bus: Mutex<NoopRawMutex, _> = Mutex::new(RefCell::new(spi));

    let display_spi = SpiDeviceWithConfig::new(&spi_bus, Output::new(display_cs, Level::High), display_config);
    let dcx = Output::new(dcx, Level::Low);
    let rst = Output::new(rst, Level::Low);

    // Display Interface abstraction from SPI and DC
    let di = SPIDeviceInterface::new(display_spi, dcx);

    // create display driver
    let mut display = Builder::st7789(di)
        .with_display_size(H as u16, W as u16)
        .with_framebuffer_size(H as u16, W as u16)
        .with_orientation(mipidsi::Orientation::Landscape(true))
        .with_invert_colors(mipidsi::ColorInversion::Inverted)
        .init(&mut Delay, Some(rst))
        .unwrap();


    // Snake styles
    let snake_style = PrimitiveStyleBuilder::new()
        .stroke_color(Rgb565::WHITE)
        .stroke_width(1)
        .fill_color(Rgb565::WHITE)
        .build();

    let blank_style = PrimitiveStyleBuilder::new()
        .stroke_color(Rgb565::BLACK)
        .stroke_width(1)
        .fill_color(Rgb565::BLACK)
        .build();

    let food_style = PrimitiveStyleBuilder::new()
        .stroke_color(Rgb565::RED)
        .stroke_width(1)
        .fill_color(Rgb565::RED)
        .build();

    let white_text_style = MonoTextStyle::new(&FONT_10X20, Rgb565::WHITE);
    let red_text_style = MonoTextStyle::new(&FONT_10X20, Rgb565::RED);

    // State of snake
    let mut gamestate = GameState::Menu;
    let mut length = 3;
    let mut direction = Direction::Right;
    let mut snake_queue: Queue<GameGrid, 512> = Queue::new();
    // These will get reset at game start
    let mut snake_head = GameGrid{x: 0, y: 0};
    let mut food = GameGrid{x: 0, y: 0};

    // Enable LCD backlight
    let mut bl = Output::new(bl, Level::High);

    // clear display
    display.clear(Rgb565::BLACK).unwrap();
    loop {
        match gamestate {
            GameState::Menu => {
                Text::new("(A) New Game", Point::new(50, 100), white_text_style)
                    .draw(&mut display)
                    .unwrap();
                if btn_a.is_low() {
                    gamestate = GameState::NewGame;
                    continue;
                }
            },
            GameState::NewGame => {
                // ensure snake queue is empty
                while !snake_queue.is_empty() {
                    _ = snake_queue.dequeue().unwrap();
                }
                snake_queue.enqueue(GameGrid{x: 10, y: 12}).unwrap();
                snake_queue.enqueue(GameGrid{x: 11, y: 12}).unwrap();
                snake_queue.enqueue(GameGrid{x: 12, y: 12}).unwrap();

                direction = Direction::Right;
                length = 3;
                snake_head = GameGrid{x: 12, y: 12};
                food = GameGrid{
                    x: (rng.next_u32() % 24) as i16,
                    y: (rng.next_u32() % 24) as i16,
                };
                gamestate = GameState::Starting;
                continue;
            },
            GameState::Starting => {
                display.clear(Rgb565::BLACK).unwrap();
                for segment in snake_queue.iter() {
                    let GameGrid{x: seg_x, y: seg_y} = segment.clone();
                    Rectangle::new(Point::new((10 * seg_x).into(), (10 * seg_y).into()), Size::new(10, 10))
                        .into_styled(snake_style)
                        .draw(&mut display)
                        .unwrap();
                }
                gamestate = GameState::Playing;
                continue;
            },
            GameState::Paused => {
                Text::new("(Y) Paused", Point::new(60, 100), white_text_style)
                    .draw(&mut display)
                    .unwrap();
                if btn_y.is_low() {
                    gamestate = GameState::Starting;
                    continue;
                }
            },
            GameState::GameOver => {
                Text::new("Game Over!!", Point::new(60, 60), red_text_style)
                    .draw(&mut display)
                    .unwrap();

                Text::new("Length:", Point::new(70, 100), white_text_style).draw(&mut display).unwrap();
                let mut score_text = ArrayString::<4>::new();
                write!(&mut score_text, "{}", length).unwrap();
                Text::new(&score_text, Point::new(150, 100), red_text_style).draw(&mut display).unwrap();

                Text::new("(A)  New Game", Point::new(50, 140), white_text_style)
                    .draw(&mut display)
                    .unwrap();
                if btn_a.is_low() {
                    gamestate = GameState::NewGame;
                    continue;
                }
                if btn_y.is_low() {
                    // exit the game completely
                    break;
                }
            },
            GameState::Playing => {
                // check button presses
                if btn_u.is_low() && !(direction == Direction::Down){
                    direction = Direction::Up;
                }
                else if btn_d.is_low() && !(direction == Direction::Up){
                    direction = Direction::Down;
                }
                else if btn_l.is_low() && !(direction == Direction::Right){
                    direction = Direction::Left;
                }
                else if btn_r.is_low() && !(direction == Direction::Left){
                    direction = Direction::Right;
                }
                if btn_y.is_low() {
                    // Change to Paused
                    gamestate = GameState::Paused;
                    continue;
                }

                // draw the snake
                let GameGrid {x: head_x, y: head_y} = snake_head.clone();

                // update the snake head
                let (head_x, head_y) = match direction {
                    Direction::Up => (head_x, head_y - 1),
                    Direction::Down => (head_x, head_y + 1),
                    Direction::Left => (head_x - 1, head_y),
                    Direction::Right => (head_x + 1, head_y),
                };

                // Check for crash
                // TODO do self collision
                if head_x < 0 || head_x >= 24 || head_y < 0 || head_y >= 24 {
                    gamestate = GameState::GameOver;
                    continue;
                }

                // draw the new head
                Rectangle::new(Point::new((10 * head_x).into(), (10 * head_y).into()), Size::new(10, 10))
                    .into_styled(snake_style)
                    .draw(&mut display)
                    .unwrap();


                // add the head to the queue
                snake_head.x = head_x;
                snake_head.y = head_y;
                snake_queue.enqueue(snake_head.clone()).unwrap();

                // check if we ate food
                if snake_head == food {
                    length += 1;
                    food = GameGrid{
                        x: (rng.next_u32() % 24) as i16,
                        y: (rng.next_u32() % 24) as i16,
                    };
                } else {
                    // dequeue the tail and blank
                    let snake_tail = snake_queue.dequeue().unwrap();
                    let GameGrid{x: tail_x, y: tail_y} = snake_tail;
                    Rectangle::new(Point::new((10 * tail_x).into(), (10 * tail_y).into()), Size::new(10, 10))
                        .into_styled(blank_style)
                        .draw(&mut display)
                        .unwrap();
                }

                // draw food
                let GameGrid{x: food_x, y: food_y} = food.clone();
                Rectangle::new(Point::new((10 * food_x).into(), (10 * food_y).into()), Size::new(10, 10))
                    .into_styled(food_style)
                    .draw(&mut display)
                    .unwrap();

                // wait 100ms
                Timer::after(Duration::from_millis(100)).await;
            },
        }
    }
    // blank screen
    display.clear(Rgb565::BLACK).unwrap();
    bl.set_low();
}

#[derive(PartialEq)]
enum Direction {
    Left,
    Right,
    Up,
    Down,
}

#[derive(Clone, Debug, PartialEq)]
struct GameGrid {
    x: i16,
    y: i16,
}

enum GameState {
    Menu,
    NewGame,
    Starting,
    Paused,
    Playing,
    GameOver,
}
