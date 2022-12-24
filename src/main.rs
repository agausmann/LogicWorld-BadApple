use std::{
    env::args_os,
    fs::{read_dir, File},
    io::{BufReader, BufWriter, Write},
    path::{Path, PathBuf},
    process::exit,
};

use anyhow::{anyhow, bail};
use blotter::{
    sandbox::{
        component::{ChubbySocket, CircuitBoard, Delayer, Peg},
        ComponentId, PegAddress, PegType, Sandbox,
    },
    BlotterFile,
};
use image::{DynamicImage, GenericImageView, ImageBuffer, Pixel, Rgb, Rgba};

fn main() -> anyhow::Result<()> {
    let path = match args_os().nth(1) {
        Some(x) => x,
        None => {
            eprintln!("missing argument `path`");
            exit(1);
        }
    };

    let mut reader = BufReader::new(File::open(&path)?);
    let file = BlotterFile::read(&mut reader)
        .map_err(|e| anyhow!("cannot parse blotter file: {:?}", e))?;

    let mut sandbox = Sandbox::from(&file.migrate());
    inject(&mut sandbox)?;
    let file = BlotterFile::V6((&sandbox).into());

    let mut writer = BufWriter::new(File::create(&path)?);
    file.write(&mut writer)
        .map_err(|e| anyhow!("cannot write blotter file: {:?}", e))?;
    writer.flush()?;

    Ok(())
}

fn inject(sandbox: &mut Sandbox) -> anyhow::Result<()> {
    let frames_dir = Path::new("frames");
    let mut frame_files: Vec<PathBuf> = read_dir(frames_dir)?
        .map(|result| result.map(|dir_entry| dir_entry.path()))
        .collect::<Result<_, _>>()?;
    frame_files.sort();

    let first_frame = image::open(&frame_files[0])?;
    let width = first_frame.width() as usize;
    let height = first_frame.height() as usize;
    drop(first_frame);

    // Two delayers for each frame (signal rise + fall)
    let depth = frame_files.len() * 2 + 1;

    let board_width: u32 = 1 + 3 * u32::try_from(width)?;
    let board_depth: u32 = 2 * u32::try_from(depth)?;

    let row_boards: Vec<ComponentId> = (0..height)
        .map(|y| {
            sandbox.add_component(
                &CircuitBoard::new()
                    .width(board_width)
                    .height(board_depth)
                    .color([51, 51, 51])
                    .build()
                    .position([0, y as i32 * 900, 0]),
            )
        })
        .collect();

    let mut row_frame_delayers = Vec::new();

    for y in 0..height {
        let mut frame_delayers = Vec::new();
        for z in 0..depth {
            // Subtract a tick from timing delayers that correspond to chunking delayers.
            let chunk_compensation = if (z + 1) % 400 == 0 { 1 } else { 0 };

            frame_delayers.push(
                sandbox.add_component(
                    &Delayer::new()
                        .delay(10 - chunk_compensation)
                        .build()
                        .parent(Some(row_boards[y]))
                        .position([150, 150, z as i32 * 600 + 150]),
                ),
            );
        }
        for z in 1..depth {
            sandbox
                .add_wire(
                    PegAddress {
                        component: frame_delayers[z - 1],
                        peg_type: PegType::Output,
                        peg_index: 0,
                    },
                    PegAddress {
                        component: frame_delayers[z],
                        peg_type: PegType::Input,
                        peg_index: 0,
                    },
                    0.0,
                )
                .unwrap();
        }
        row_frame_delayers.push(frame_delayers);
    }

    let mut row_col_last_pegs = Vec::new();
    for y in 0..height {
        let mut col_last_pegs = Vec::new();
        for x in 0..width {
            col_last_pegs.push(
                sandbox.add_component(
                    &ChubbySocket::new()
                        .build()
                        .parent(Some(row_boards[y]))
                        .position([x as i32 * 900 + 750, 150, 150])
                        .rotation([0.0, 1.0, 0.0, 0.0]),
                ),
            );
        }
        row_col_last_pegs.push(col_last_pegs);
    }

    let mut last_frame = DynamicImage::ImageRgb8(ImageBuffer::from_pixel(
        width as u32,
        height as u32,
        Rgb([0, 0, 0]),
    ));

    for (frame_index, path) in frame_files.iter().enumerate() {
        eprintln!("{}", frame_index);
        let z = (frame_index + 1) * 2;
        let current_frame = image::open(path)?;
        if current_frame.width() as usize != width || current_frame.height() as usize != height {
            bail!("{:?}: frame does not match size of first frame", path);
        }

        // Force inserting a delayer every once in a while, to "chunk" the huge nets made
        // by pixel signal wires and effectively reduce UPS.
        // The additional delay caused by these delayers is compensated for in the timing delayers.
        let at_chunk_boundary = (frame_index + 1) % 200 == 0;
        if at_chunk_boundary {
            for y in 0..height {
                for x in 0..width {
                    let chunk_delayer = sandbox.add_component(
                        &Delayer::new()
                            .delay(1)
                            .build()
                            .parent(Some(row_boards[y]))
                            .position([x as i32 * 900 + 750, 150, z as i32 * 600 - 450])
                            .rotation([0.0, 1.0, 0.0, 0.0]),
                    );
                    sandbox
                        .add_wire(
                            PegAddress {
                                component: chunk_delayer,
                                peg_type: PegType::Output,
                                peg_index: 0,
                            },
                            PegAddress {
                                component: row_col_last_pegs[y][x],
                                peg_type: PegType::Input,
                                peg_index: 0,
                            },
                            0.0,
                        )
                        .unwrap();
                }
            }
        }

        for y in 0..height {
            let mut row_last_delayer = row_frame_delayers[y][z];
            for x in 0..width {
                let last_pixel = to_1bit(last_frame.get_pixel(x as u32, (height - 1 - y) as u32));
                let current_pixel =
                    to_1bit(current_frame.get_pixel(x as u32, (height - 1 - y) as u32));
                if current_pixel != last_pixel {
                    let pixel_delayer = sandbox.add_component(
                        &Delayer::new()
                            .delay(1)
                            .build()
                            .parent(Some(row_boards[y]))
                            .position([x as i32 * 900 - 450, 150, z as i32 * 600 - 150])
                            .rotation([0.0, 1.0, 0.0, 0.0]),
                    );

                    let pixel_peg;
                    // Chunking delayers replace the pegs that would usually be generated:
                    if at_chunk_boundary {
                        pixel_peg = row_col_last_pegs[y][x];
                    } else {
                        pixel_peg = sandbox.add_component(
                            &Peg::new().build().parent(Some(row_boards[y])).position([
                                x as i32 * 900 + 750,
                                150,
                                z as i32 * 600 - 450,
                            ]),
                        );
                    }

                    sandbox
                        .add_wire(
                            PegAddress {
                                component: row_last_delayer,
                                peg_type: PegType::Input,
                                peg_index: 0,
                            },
                            PegAddress {
                                component: pixel_delayer,
                                peg_type: PegType::Input,
                                peg_index: 0,
                            },
                            0.0,
                        )
                        .unwrap();
                    sandbox
                        .add_wire(
                            PegAddress {
                                component: pixel_delayer,
                                peg_type: PegType::Output,
                                peg_index: 0,
                            },
                            PegAddress {
                                component: pixel_peg,
                                peg_type: PegType::Input,
                                peg_index: 0,
                            },
                            0.0,
                        )
                        .unwrap();

                    // This wire is not needed if using a chunking delayer
                    if !at_chunk_boundary {
                        sandbox
                            .add_wire(
                                PegAddress {
                                    component: pixel_peg,
                                    peg_type: PegType::Input,
                                    peg_index: 0,
                                },
                                PegAddress {
                                    component: row_col_last_pegs[y][x],
                                    peg_type: PegType::Input,
                                    peg_index: 0,
                                },
                                0.0,
                            )
                            .unwrap();
                    }

                    row_last_delayer = pixel_delayer;
                    row_col_last_pegs[y][x] = pixel_peg;
                }
            }
        }

        last_frame = current_frame;
    }

    Ok(())
}

fn to_1bit(pixel: Rgba<u8>) -> bool {
    pixel.to_luma().0[0] > 127
}
