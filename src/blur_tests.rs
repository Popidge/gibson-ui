use super::*;

#[test]
fn postprocess_scissor_covers_odd_panes_and_clamps_padding() {
    let pane = Rect {
        x: 101.0,
        y: 53.0,
        width: 701.0,
        height: 399.0,
    };
    assert_eq!(
        padded_scaled_scissor(pane, 1001, 601, 501, 301, 10),
        (40, 16, 372, 221)
    );
    assert_eq!(
        padded_scaled_scissor(
            Rect {
                x: 0.0,
                y: 0.0,
                width: 1.0,
                height: 1.0
            },
            1,
            1,
            1,
            1,
            10
        ),
        (0, 0, 1, 1)
    );
}

#[test]
#[ignore = "requires a graphics adapter; compares paired blur pixels and GPU timings"]
fn paired_blur_matches_reference_on_gpu() {
    pollster::block_on(async {
        let adapter = wgpu::Instance::default()
            .request_adapter(&Default::default())
            .await
            .unwrap();
        let timestamps = adapter.features().contains(wgpu::Features::TIMESTAMP_QUERY);
        eprintln!("BLUR GPU {:?}", adapter.get_info());
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                required_features: if timestamps {
                    wgpu::Features::TIMESTAMP_QUERY
                } else {
                    wgpu::Features::empty()
                },
                ..Default::default()
            })
            .await
            .unwrap();
        let width = 960;
        let height = 540;
        let make_texture = || {
            device.create_texture(&wgpu::TextureDescriptor {
                label: None,
                size: wgpu::Extent3d {
                    width,
                    height,
                    depth_or_array_layers: 1,
                },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING
                    | wgpu::TextureUsages::COPY_DST
                    | wgpu::TextureUsages::RENDER_ATTACHMENT
                    | wgpu::TextureUsages::COPY_SRC,
                view_formats: &[],
            })
        };
        let source = make_texture();
        // Fine texture, hard edges and isolated highlights exercise interpolation.
        let pixels: Vec<u8> = (0..width * height)
            .flat_map(|i| {
                let n = i.wrapping_mul(1664525).wrapping_add(1013904223);
                [
                    (n >> 24) as u8,
                    if i % 97 == 0 { 255 } else { 0 },
                    if i % width < width / 2 { 255 } else { 0 },
                    255,
                ]
            })
            .collect();
        queue.write_texture(
            source.as_image_copy(),
            &pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
            source.size(),
        );
        let source_view = source.create_view(&Default::default());
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let shader_source = include_str!("blur.wgsl");
        let reference = shader_source.replace(
            "return unit_blur(input.uv, vec2<f32>(0.0, 1.0));",
            "return bloom_blur(input.uv, vec2<f32>(0.0, 1.0));",
        );
        let mut outputs = Vec::new();
        for (name, text) in [("dense", reference.as_str()), ("paired", shader_source)] {
            let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                label: Some(name),
                source: wgpu::ShaderSource::Wgsl(text.into()),
            });
            let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(name),
                layout: None,
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_bloom_vertical"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: wgpu::TextureFormat::Rgba8Unorm,
                        blend: None,
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: Default::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            });
            let group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &pipeline.get_bind_group_layout(0),
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(&source_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&sampler),
                    },
                ],
            });
            let target = make_texture();
            let view = target.create_view(&Default::default());
            let readback = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: (width * height * 4) as u64,
                usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
                mapped_at_creation: false,
            });
            let queries = timestamps.then(|| {
                device.create_query_set(&wgpu::QuerySetDescriptor {
                    label: None,
                    ty: wgpu::QueryType::Timestamp,
                    count: 2,
                })
            });
            let resolve = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: 16,
                usage: wgpu::BufferUsages::QUERY_RESOLVE | wgpu::BufferUsages::COPY_SRC,
                mapped_at_creation: false,
            });
            let times = device.create_buffer(&wgpu::BufferDescriptor {
                label: None,
                size: 16,
                usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            // Warm the pipeline, then sample multiple batches to reduce clock noise.
            let mut timings = Vec::new();
            for batch in 0..6 {
                let mut encoder = device.create_command_encoder(&Default::default());
                {
                    let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                        label: Some(name),
                        color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                            view: &view,
                            depth_slice: None,
                            resolve_target: None,
                            ops: wgpu::Operations {
                                load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                                store: wgpu::StoreOp::Store,
                            },
                        })],
                        depth_stencil_attachment: None,
                        timestamp_writes: queries.as_ref().map(|query_set| {
                            wgpu::RenderPassTimestampWrites {
                                query_set,
                                beginning_of_pass_write_index: Some(0),
                                end_of_pass_write_index: Some(1),
                            }
                        }),
                        occlusion_query_set: None,
                        multiview_mask: None,
                    });
                    pass.set_pipeline(&pipeline);
                    pass.set_bind_group(0, &group, &[]);
                    pass.draw(0..3, 0..32);
                }
                if let Some(queries) = &queries {
                    encoder.resolve_query_set(queries, 0..2, &resolve, 0);
                    encoder.copy_buffer_to_buffer(&resolve, 0, &times, 0, 16);
                }
                encoder.copy_texture_to_buffer(
                    target.as_image_copy(),
                    wgpu::TexelCopyBufferInfo {
                        buffer: &readback,
                        layout: wgpu::TexelCopyBufferLayout {
                            offset: 0,
                            bytes_per_row: Some(width * 4),
                            rows_per_image: Some(height),
                        },
                    },
                    target.size(),
                );
                queue.submit([encoder.finish()]);
                let read = |buffer: &wgpu::Buffer| {
                    let (tx, rx) = std::sync::mpsc::channel();
                    buffer.map_async(wgpu::MapMode::Read, .., move |result| {
                        tx.send(result).unwrap();
                    });
                    device.poll(wgpu::PollType::wait_indefinitely()).unwrap();
                    rx.recv().unwrap().unwrap();
                    let data = buffer.get_mapped_range(..).unwrap().to_vec();
                    buffer.unmap();
                    data
                };
                if timestamps {
                    let data = read(&times);
                    let values: &[u64] = bytemuck::cast_slice(&data);
                    if batch > 0 {
                        timings.push(
                            (values[1] - values[0]) as f64 * queue.get_timestamp_period() as f64
                                / 32e6,
                        );
                    }
                }
                let data = read(&readback);
                if batch == 5 {
                    outputs.push(data);
                }
            }
            timings.sort_by(f64::total_cmp);
            if !timings.is_empty() {
                eprintln!("BLUR {name} median_ms={:.4}", timings[timings.len() / 2]);
            }
        }
        let differences: Vec<_> = outputs[0]
            .iter()
            .zip(&outputs[1])
            .map(|(a, b)| a.abs_diff(*b))
            .collect();
        let max = *differences.iter().max().unwrap();
        let mean = differences.iter().map(|&d| d as f64).sum::<f64>() / differences.len() as f64;
        eprintln!("BLUR pixel difference max={max}/255 mean={mean:.4}/255");
        assert!(max <= 2 && mean < 0.2, "paired blur changed the image");
    });
}
