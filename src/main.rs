use std::{
    collections::HashMap,
    env::args_os,
    fs::{read_dir, File},
    io::{BufReader, BufWriter, Write},
    iter::repeat_with,
    path::{Path, PathBuf},
    process::exit,
};

use anyhow::bail;
use blotter::{BlotterFile, CircuitStates, Component, Input, Output, PegAddress, Wire};
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
    let mut file = BlotterFile::read(&mut reader)?;

    inject(&mut file)?;

    let mut writer = BufWriter::new(File::create(&path)?);
    file.write(&mut writer)?;
    writer.flush()?;

    Ok(())
}

fn inject(file: &mut BlotterFile) -> anyhow::Result<()> {
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

    let component_id_map: HashMap<&str, u16> = file
        .component_types
        .iter()
        .map(|ty| (ty.text_id.as_str(), ty.numeric_id))
        .collect();

    let mhg_circuit_board = component_id_map["MHG.CircuitBoard"];
    let mhg_delayer = component_id_map["MHG.Delayer"];
    let mhg_peg = component_id_map["MHG.Peg"];
    let mhg_chubby_socket = component_id_map["MHG.ChubbySocket"];

    let mut last_addr = file
        .components
        .iter()
        .map(|comp| comp.address)
        .max()
        .unwrap_or(0);
    let mut get_addr = || {
        last_addr += 1;
        last_addr
    };
    let mut last_cluster = file
        .components
        .iter()
        .flat_map(|comp| {
            (comp.inputs.iter().map(|inp| inp.circuit_state_id))
                .chain(comp.outputs.iter().map(|out| out.circuit_state_id))
                .max()
        })
        .max()
        .unwrap_or(0);
    let mut get_cluster = || {
        last_cluster += 1;
        last_cluster
    };

    let row_boards: Vec<u32> = repeat_with(&mut get_addr).take(height).collect();
    let board_width: i32 = 1 + 3 * i32::try_from(width)?;
    let board_depth: i32 = 2 * i32::try_from(depth)?;

    for y in 0..height {
        let mut board_data = vec![0; 11];
        board_data[0..3].copy_from_slice(&[51, 51, 51]);
        board_data[3..7].copy_from_slice(&board_width.to_le_bytes());
        board_data[7..11].copy_from_slice(&board_depth.to_le_bytes());
        file.components.push(Component {
            address: row_boards[y],
            parent: 0,
            type_id: mhg_circuit_board,
            position: [0.0, y as f32 * 0.90, 0.0],
            rotation: [0.0, 0.0, 0.0, 1.0],
            inputs: vec![],
            outputs: vec![],
            custom_data: Some(board_data),
        });
    }

    let mut row_frame_clusters = Vec::new();
    let mut row_frame_delayers = Vec::new();

    for y in 0..height {
        let frame_clusters: Vec<i32> = repeat_with(&mut get_cluster).take(depth + 1).collect();
        let frame_delayers: Vec<u32> = repeat_with(&mut get_addr).take(depth).collect();
        for z in 0..depth {
            // Subtract a tick from timing delayers that correspond to chunking delayers.
            let chunk_compensation = if (z + 1) % 400 == 0 { 1 } else { 0 };

            file.components.push(Component {
                address: frame_delayers[z as usize],
                parent: row_boards[y],
                type_id: mhg_delayer,
                position: [0.15, 0.15, z as f32 * 0.60 + 0.15],
                rotation: [0.0, 0.0, 0.0, 1.0],
                inputs: vec![Input {
                    circuit_state_id: frame_clusters[z],
                }],
                outputs: vec![Output {
                    circuit_state_id: frame_clusters[z + 1],
                }],
                custom_data: Some(vec![0, 0, 0, 0, 10 - chunk_compensation, 0, 0, 0]),
            });
        }
        for z in 1..depth {
            file.wires.push(Wire {
                start_peg: PegAddress {
                    is_input: false,
                    component_address: frame_delayers[z - 1],
                    peg_index: 0,
                },
                end_peg: PegAddress {
                    is_input: true,
                    component_address: frame_delayers[z],
                    peg_index: 0,
                },
                circuit_state_id: frame_clusters[z],
                rotation: 0.0,
            })
        }
        row_frame_clusters.push(frame_clusters);
        row_frame_delayers.push(frame_delayers);
    }

    let mut row_col_clusters = Vec::new();
    for _y in 0..height {
        let row_clusters: Vec<i32> = repeat_with(&mut get_cluster).take(width).collect();
        row_col_clusters.push(row_clusters);
    }

    let mut row_col_last_pegs = Vec::new();
    for y in 0..height {
        let col_last_pegs: Vec<u32> = repeat_with(&mut get_addr).take(width).collect();
        for x in 0..width {
            file.components.push(Component {
                address: col_last_pegs[x],
                parent: row_boards[y],
                type_id: mhg_chubby_socket,
                position: [x as f32 * 0.90 + 0.75, 0.15, 0.15],
                rotation: [0.0, 1.0, 0.0, 0.0],
                inputs: vec![Input {
                    circuit_state_id: row_col_clusters[y][x],
                }],
                outputs: vec![],
                custom_data: None,
            });
        }
        row_col_last_pegs.push(col_last_pegs);
    }

    let mut last_frame = DynamicImage::ImageRgb8(ImageBuffer::from_pixel(
        width as u32,
        height as u32,
        Rgb([0, 0, 0]),
    ));

    for (frame_index, path) in frame_files.iter().enumerate() {
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
                    let chunk_delayer = get_addr();
                    let new_cluster = get_cluster();
                    file.components.push(Component {
                        address: chunk_delayer,
                        parent: row_boards[y],
                        type_id: mhg_delayer,
                        position: [x as f32 * 0.90 + 0.75, 0.15, z as f32 * 0.60 - 0.45],
                        rotation: [0.0, 1.0, 0.0, 0.0],
                        inputs: vec![Input {
                            circuit_state_id: new_cluster,
                        }],
                        outputs: vec![Output {
                            circuit_state_id: row_col_clusters[y][x],
                        }],
                        custom_data: Some(vec![0, 0, 0, 0, 1, 0, 0, 0]),
                    });
                    file.wires.push(Wire {
                        start_peg: PegAddress {
                            is_input: false,
                            component_address: chunk_delayer,
                            peg_index: 0,
                        },
                        end_peg: PegAddress {
                            is_input: true,
                            component_address: row_col_last_pegs[y][x],
                            peg_index: 0,
                        },
                        circuit_state_id: row_col_clusters[y][x],
                        rotation: 0.0,
                    });
                    row_col_last_pegs[y][x] = chunk_delayer;
                    row_col_clusters[y][x] = new_cluster;
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
                    let pixel_delayer = get_addr();

                    file.components.push(Component {
                        address: pixel_delayer,
                        parent: row_boards[y],
                        type_id: mhg_delayer,
                        position: [x as f32 * 0.90 + 0.45, 0.15, z as f32 * 0.60 - 0.15],
                        rotation: [0.0, 1.0, 0.0, 0.0],
                        inputs: vec![Input {
                            circuit_state_id: row_frame_clusters[y][z],
                        }],
                        outputs: vec![Output {
                            circuit_state_id: row_col_clusters[y][x],
                        }],
                        custom_data: Some(vec![0, 0, 0, 0, 1, 0, 0, 0]),
                    });

                    let pixel_peg;
                    // Chunking delayers replace the pegs that would usually be generated:
                    if at_chunk_boundary {
                        pixel_peg = row_col_last_pegs[y][x];
                    } else {
                        pixel_peg = get_addr();
                        file.components.push(Component {
                            address: pixel_peg,
                            parent: row_boards[y],
                            type_id: mhg_peg,
                            position: [x as f32 * 0.90 + 0.75, 0.15, z as f32 * 0.60 - 0.45],
                            rotation: [0.0, 0.0, 0.0, 1.0],
                            inputs: vec![Input {
                                circuit_state_id: row_col_clusters[y][x],
                            }],
                            outputs: vec![],
                            custom_data: None,
                        });
                    }

                    file.wires.push(Wire {
                        start_peg: PegAddress {
                            is_input: true,
                            component_address: row_last_delayer,
                            peg_index: 0,
                        },
                        end_peg: PegAddress {
                            is_input: true,
                            component_address: pixel_delayer,
                            peg_index: 0,
                        },
                        circuit_state_id: row_frame_clusters[y][z],
                        rotation: 0.0,
                    });

                    file.wires.push(Wire {
                        start_peg: PegAddress {
                            is_input: false,
                            component_address: pixel_delayer,
                            peg_index: 0,
                        },
                        end_peg: PegAddress {
                            is_input: true,
                            component_address: pixel_peg,
                            peg_index: 0,
                        },
                        circuit_state_id: row_col_clusters[y][x],
                        rotation: 0.0,
                    });

                    // This wire is not needed if using a chunking delayer
                    if !at_chunk_boundary {
                        file.wires.push(Wire {
                            start_peg: PegAddress {
                                is_input: true,
                                component_address: pixel_peg,
                                peg_index: 0,
                            },
                            end_peg: PegAddress {
                                is_input: true,
                                component_address: row_col_last_pegs[y][x],
                                peg_index: 0,
                            },
                            circuit_state_id: row_col_clusters[y][x],
                            rotation: 0.0,
                        });
                    }

                    row_last_delayer = pixel_delayer;
                    row_col_last_pegs[y][x] = pixel_peg;
                }
            }
        }

        last_frame = current_frame;
    }

    let num_clusters = usize::try_from(last_cluster)? + 1;
    match &mut file.circuit_states {
        CircuitStates::WorldFormat { circuit_states } => {
            // Fill to zeros
            circuit_states.resize((num_clusters - 1) / 8 + 1, 0);
        }
        CircuitStates::SubassemblyFormat { .. } => {}
    }
    Ok(())
}

fn to_1bit(pixel: Rgba<u8>) -> bool {
    pixel.to_luma().0[0] > 127
}
