use eframe::{egui::{self}, App, Frame, NativeOptions};
use id3::{Tag, TagLike, Version};
use id3::frame::{Picture, PictureType};
use symphonia::default::{get_probe};
use symphonia::core::{
    codecs::CodecParameters,
    formats::FormatOptions,
    io::MediaSourceStream,
    meta::MetadataOptions,
    probe::Hint,
};
use std::collections::HashMap;
use std::path::Path;
use std::fs::File;

struct MyApp {
    cached_tag: Option<Tag>,
    dropped_files: Vec<String>,
    selected_file: Option<String>,
    alert_message: String,
    selected_album_art: Option<egui::TextureId>,
    album_art_cache: HashMap<String, egui::TextureId>,
    album_art_ready: bool,
    editing_artist: bool,
    edited_artist: String,
    editing_title: bool,
    edited_title: String,
    editing_album: bool,
    edited_album: String,
    editing_genre: bool,
    edited_genre: String,
}

impl MyApp {
    fn pick_and_set_album_art(&mut self, ctx: &egui::Context) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(path) = rfd::FileDialog::new()
            .add_filter("Image", &["png", "jpg", "jpeg"])
            .pick_file()
        {
            let image = image::open(&path)?.to_rgba8();
            let size = [image.width() as usize, image.height() as usize];
            let pixels = image.into_raw();

            let color_image = egui::ColorImage::from_rgba_unmultiplied(size, &pixels);

            let texture_id = ctx
                .tex_manager()
                .write()
                .alloc("album_art".into(), color_image.into(), Default::default());


            self.selected_album_art = Some(texture_id);

            if let Some(song_path) = &self.selected_file {
                self.album_art_cache.insert(song_path.clone(), texture_id);
                let mut tag = Tag::read_from_path(song_path)?;

                tag.remove_all_pictures();

                let img_bytes = std::fs::read(&path)?;

                let picture = Picture {
                    mime_type: "image/png".to_string(),
                    picture_type: PictureType::CoverFront,
                    description: "".to_string(),
                    data: img_bytes,
                };

                let frame = id3::Frame::with_content("APIC", id3::Content::Picture(picture));
                tag.add_frame(frame);
                tag.write_to_path(song_path, Version::Id3v24)?;

                self.cached_tag = Some(tag);
            }

            Ok(())
        } else {
            Ok(())
        }
    }

    fn get_file_name(file: &String) -> &str {
        Path::new(file)
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("Unknown")
    }

    fn get_tag(&mut self, path: &str) -> Result<&Tag, Box<dyn std::error::Error>> {
        if self.cached_tag.is_none() {
            let tag = Tag::read_from_path(path)?;
            self.cached_tag = Some(tag);
        }
        Ok(self.cached_tag.as_ref().unwrap())
    }

    fn get_title(&mut self, path: &str) -> Result<Option<String>, Box<dyn std::error::Error>> {
        Ok(self.get_tag(path)?.title().map(|s| s.to_string()))
    }

    fn set_title(&mut self, path: &str, title: String) -> Result<(), Box<dyn std::error::Error>> {
        let mut tag = Tag::read_from_path(path)?;
        tag.set_title(title);
        tag.write_to_path(path, Version::Id3v24)?;
        self.cached_tag = Some(tag);
        Ok(())
    }

    fn get_artist(&mut self, path: &str) -> Result<Option<String>, Box<dyn std::error::Error>> {
        if let Some(artist_str) = self.get_tag(path)?.artist() {
            let clean_str = artist_str.replace('\0', ";").replace('\r', ";").replace('\n', ";").replace(',', ";");
            let artists: Vec<&str> = clean_str
                .split(';')
                .map(|s| s.trim())
                .filter(|s| !s.is_empty())
                .collect();
            let joined = artists.join(", ");
            Ok(Some(joined))
        } else {
            Ok(None)
        }
    }


    fn set_artist(&mut self, path: &str, artist: String) -> Result<(), Box<dyn std::error::Error>> {
        let mut tag = Tag::read_from_path(path)?;
        tag.set_artist(artist);
        tag.write_to_path(path, Version::Id3v24)?;
        self.cached_tag = Some(tag);
        Ok(())
    }

    fn get_album(&mut self, path: &str) -> Result<Option<String>, Box<dyn std::error::Error>> {
        Ok(self.get_tag(path)?.album().map(|s| s.to_string()))
    }

    fn set_album(&mut self, path: &str, album: String) -> Result<(), Box<dyn std::error::Error>> {
        let mut tag = Tag::read_from_path(path)?;
        tag.set_album(album);
        tag.write_to_path(path, Version::Id3v24)?;
        self.cached_tag = Some(tag);
        Ok(())
    }

    fn get_album_art(&mut self, path: &str) -> Result<Option<(Vec<u8>, String)>, Box<dyn std::error::Error>> {
        let tag = self.get_tag(path)?;
        if let Some(picture) = tag.pictures().next() {
            Ok(Some((picture.data.clone(), picture.mime_type.clone())))
        } else {
            Ok(None)
        }
    }

    fn save_album_art(&mut self, path: &str) -> Result<(), Box<dyn std::error::Error>> {
        if let Some((bytes, mime)) = self.get_album_art(path)? {
            // determine extension from mime type
            let ext = match mime.as_str() {
                "image/png" => "png",
                "image/jpeg" | "image/jpg" => "jpg",
                _ => "bin", // fallback
            };

            if let Some(save_path) = rfd::FileDialog::new()
                .set_file_name(&format!("artwork.{}", ext))
                .save_file()
            {
                std::fs::write(save_path, &bytes)?;
            }
        } else {
            return Err("No album art found".into());
        }
        Ok(())
    }

    fn load_album_art_texture(
        &mut self,
        ctx: &egui::Context,
        path: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        if let Some(tex) = self.album_art_cache.get(path) {
            self.selected_album_art = Some(*tex);
            return Ok(());
        }

        if let Some((bytes, _mime)) = self.get_album_art(path)? {
            let start_decode = std::time::Instant::now();
            let mut image = image::load_from_memory(&bytes)?;
            let decode_duration = start_decode.elapsed();
            println!("Decoding image took: {:?}", decode_duration);

            let max_size = 512;
            let start_resize = std::time::Instant::now();
            if image.width() > max_size || image.height() > max_size {
                image = image::DynamicImage::ImageRgba8(image::imageops::thumbnail(&image, max_size, max_size));
            }
            let resize_duration = start_resize.elapsed();
            println!("Resizing image took: {:?}", resize_duration);

            let size = [image.width() as usize, image.height() as usize];
            let pixels = image.to_rgba8().into_raw();

            let color_image = egui::ColorImage::from_rgba_unmultiplied(size, &pixels);

            let start_upload = std::time::Instant::now();
            let texture_id = ctx
                .tex_manager()
                .write()
                .alloc("album_art".into(), color_image.into(), Default::default());
            let upload_duration = start_upload.elapsed();
            println!("Uploading texture took: {:?}", upload_duration);

            self.album_art_cache.insert(path.to_string(), texture_id);
            self.selected_album_art = Some(texture_id);
            self.album_art_ready = true;
        } else {
            self.selected_album_art = None;
        }
        Ok(())
        
    }

    fn get_genre(&mut self, path: &str) -> Result<Option<String>, Box<dyn std::error::Error>> {
        Ok(self.get_tag(path)?.genre().map(|s| s.to_string()))
    }

    fn set_genre(&mut self, path: &str, genre: String) -> Result<(), Box<dyn std::error::Error>> {
        let mut tag = Tag::read_from_path(path)?;
        tag.set_genre(genre);
        tag.write_to_path(path, Version::Id3v24)?;
        self.cached_tag = Some(tag);
        Ok(())
    }

    fn get_codec_params<P: AsRef<Path>>(path: P) -> Result<CodecParameters, Box<dyn std::error::Error>> {
        let file = File::open(path)?;
        let mss = MediaSourceStream::new(Box::new(file), Default::default());

        let hint = Hint::new();
        let probed = get_probe().format(
            &hint,
            mss,
            &FormatOptions::default(),
            &MetadataOptions::default(),
        )?;

        let format = probed.format;
        let track = format.default_track().ok_or("No default track")?;
        Ok(track.codec_params.clone())
    }

    fn get_bitrate<P: AsRef<Path>>(path: P) -> Result<Option<u32>, Box<dyn std::error::Error>> {
        let params = Self::get_codec_params(&path)?;

        let metadata = std::fs::metadata(path)?;
        let file_size_bits = metadata.len() * 8;

        let duration = if let (Some(rate), Some(frames)) = (params.sample_rate, params.n_frames) {
            Some(frames as f64 / rate as f64)
        } else {
            None
        };

        if let Some(dur) = duration {
            if dur > 0.0 {
                let approx_bitrate = (file_size_bits as f64 / dur) as u32;
                return Ok(Some(approx_bitrate));
            }
        }

        Ok(None)
    }

    fn get_sample_rate(path: &str) -> Result<Option<u32>, Box<dyn std::error::Error>> {
        let params = Self::get_codec_params(path)?;
        Ok(params.sample_rate)
    }

    fn get_duration_seconds(path: &str) -> Result<Option<f64>, Box<dyn std::error::Error>> {
        let params = Self::get_codec_params(path)?;
        match (params.sample_rate, params.n_frames) {
            (Some(rate), Some(frames)) => Ok(Some(frames as f64 / rate as f64)),
            _ => Ok(None),
        }
    }

    fn truncate_filename_with_ext(name: &str, max_len: usize) -> String {
        if name.len() <= max_len {
            return name.to_string();
        }
        let ext = std::path::Path::new(name)
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("");
        let ext_len = ext.len() + 1;
        if max_len > ext_len + 3 {
            let prefix = &name[..max_len - ext_len - 3];
            format!("{}...{}", prefix, ext)
        } else {
            let prefix = &name[..max_len - 3];
            format!("{}...", prefix)
        }
    }

}


impl Default for MyApp {
    fn default() -> Self {
        Self {
            cached_tag: None,
            dropped_files: Vec::new(),
            selected_file: None,
            selected_album_art: None,
            album_art_cache: HashMap::new(),
            album_art_ready: false,
            alert_message: String::new(),
            edited_artist: String::new(),
            editing_artist: false,
            edited_title: String::new(),
            editing_title: false,
            editing_album: false,
            edited_album: String::new(),
            editing_genre: false,
            edited_genre: String::new(),
        }
    }
}

impl App for MyApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut Frame) {
        let mut app = MyApp::default();
        let input = ctx.input(|i| i.clone());

        egui::SidePanel::left("my_left_panel")
        .resizable(false)
        .frame(egui::Frame::default().fill(egui::Color32::from_rgb(20, 20, 20)).inner_margin(5.0))
        .default_width(200.0)
        .show(ctx, |ui| {
            ui.set_max_width(150.0);
            egui::ScrollArea::vertical()
            .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(
                egui::RichText::new("Files")
                    .size(25.0)
                    .color(egui::Color32::WHITE),
                );
                if ui.button("add").clicked() {
                    if let Some(path) = rfd::FileDialog::new()
                        .add_filter("Audio", &["mp3"])
                        .pick_file()
                    {
                        let path_str = path.display().to_string();

                        if !self.dropped_files.contains(&path_str) {
                            self.dropped_files.push(path_str);
                            self.dropped_files.sort();
                        }
                    }
                };
            });
            

            for file in &input.raw.dropped_files {
                if let Some(path) = &file.path {
                    let path_str = path.display().to_string();

                    if path.extension().and_then(|ext| ext.to_str()) == Some("mp3") {
                        if !self.dropped_files.contains(&path_str) {
                            self.dropped_files.push(path_str);
                            self.dropped_files.sort();
                        }
                        self.alert_message.clear();
                    } else {
                        self.alert_message = format!("File '{}' is not an mp3!", MyApp::get_file_name(&path_str));
                    }
                }
            }

            ui.label("Song list:");
            let mut selected_file_to_load = None;
            let mut file_to_remove: Option<String> = None;

            for file in &self.dropped_files {
                let is_selected = Some(file) == self.selected_file.as_ref();

                let button = egui::Button::new(
                    egui::RichText::new(MyApp::get_file_name(&file)).color(
                        if is_selected {
                            egui::Color32::WHITE
                        } else {
                            egui::Color32::GRAY
                        },
                    ),
                )
                .fill(if is_selected {
                    egui::Color32::from_rgb(100, 150, 255)
                } else {
                    egui::Color32::from_rgb(60, 60, 60)
                });

                let response = ui.add(button);

                if response.clicked() {
                    self.selected_file = Some(file.clone());
                    self.selected_album_art = None;
                    self.cached_tag = None;
                    selected_file_to_load = self.selected_file.clone();
                }

                response.context_menu(|ui| {
                    if ui.button("Remove").clicked() {
                        file_to_remove = Some(file.clone());
                        ui.close_menu();
                    }
                });
            }

            if let Some(file) = file_to_remove {
                self.dropped_files.retain(|f| f != &file);
                if self.selected_file.as_ref() == Some(&file) {
                    self.selected_file = None;
                }
            }

            if let Some(path) = selected_file_to_load {
                let _ = self.load_album_art_texture(ctx, &path);
            }
            });
        });


        egui::TopBottomPanel::top("top_panel")
        .frame(egui::Frame::default().fill(egui::Color32::from_rgb(20, 20, 20)).inner_margin(5.0))
        .show(ctx, |ui| {
            ui.horizontal_centered(|ui| {
                if let Some(selected) = &self.selected_file {
                    let file_name = MyApp::get_file_name(selected);
                    ui.heading(
                        egui::RichText::new(MyApp::truncate_filename_with_ext(file_name, 50))
                            .size(25.0)
                            .color(egui::Color32::WHITE),
                    );
                } else {
                    ui.heading(
                        egui::RichText::new("No File Selected")
                            .size(25.0)
                            .color(egui::Color32::WHITE),
                    );
                }
            });
        });

        egui::TopBottomPanel::bottom("alert_panel")
        .frame(egui::Frame::default().fill(egui::Color32::from_rgb(20, 20, 20)).inner_margin(5.0))
        .show(ctx, |ui| {
            if !self.alert_message.is_empty() {
                ui.colored_label(egui::Color32::RED, &self.alert_message);
            } else {
                if ui.link(egui::RichText::new("github").color(egui::Color32::WHITE).size(15.0)).clicked() {
                    let _ = open::that("https://github.com/joshjkns");
                }
            }
        });

        egui::CentralPanel::default().show(ctx, |ui| {
            let total_width = ui.available_width();
            let right_width = 300.0;
            let left_width = total_width - right_width;
            if self.selected_file.is_some() {
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Min), |ui| {
                egui::ScrollArea::vertical()
                .show(ui, |ui| {
                    ui.vertical(|ui| {
                    ui.set_width(left_width);

                    // artist
                    ui.horizontal(|ui| {
                        ui.heading("Artist:");
                        if ui.button("edit").clicked() {
                            self.editing_artist = true;
                        }
                    });

                    let artist_label = self
                        .selected_file
                        .as_ref()
                        .and_then(|path| app.get_artist(path).ok().flatten())
                        .unwrap_or_else(|| "No artist info".to_string());
                    ui.label(egui::RichText::new(artist_label).color(egui::Color32::WHITE).size(16.0));

                    if self.editing_artist {
                        egui::Window::new("Edit Artist").show(ctx, |ui| {
                            ui.text_edit_singleline(&mut self.edited_artist);
                            if ui.button("Save").clicked() {
                                if let Some(path) = &self.selected_file {
                                    let artist = self.edited_artist.clone();
                                    if let Err(e) = app.set_artist(path, artist) {
                                        self.alert_message = format!("Failed to save artist: {}", e);
                                    } else {
                                        self.editing_artist = false;
                                    }
                                }
                            }
                            if ui.button("Cancel").clicked() {
                                self.editing_artist = false;
                            }
                        });
                    }
                    ui.add_space(10.0);

                    // title
                    ui.horizontal(|ui| {
                        ui.heading("Title:");
                        if ui.button("edit").clicked() {
                            self.editing_title = true;
                        }
                    });

                    let title_label = self
                        .selected_file
                        .as_ref()
                        .and_then(|path| app.get_title(path).ok().flatten())
                        .unwrap_or_else(|| "No title info".to_string());
                    ui.label(egui::RichText::new(title_label).color(egui::Color32::WHITE).size(16.0));

                    if self.editing_title {
                        egui::Window::new("Edit Title").show(ctx, |ui| {
                            ui.text_edit_singleline(&mut self.edited_title);
                            if ui.button("Save").clicked() {
                                if let Some(path) = &self.selected_file {
                                    let title = self.edited_title.clone();
                                    if let Err(e) = app.set_title(path, title) {
                                        self.alert_message = format!("Failed to save title: {}", e);
                                    } else {
                                        self.editing_title = false;
                                    }
                                }
                            }
                            if ui.button("Cancel").clicked() {
                                self.editing_title = false;
                            }
                        });
                    }

                    ui.add_space(10.0);
                    
                    // album
                    ui.horizontal(|ui| {
                        ui.heading("Album:");
                        if ui.button("edit").clicked() {
                            self.editing_album = true;
                        }
                    });

                    let album_label = self
                        .selected_file
                        .as_ref()
                        .and_then(|path| app.get_album(path).ok().flatten())
                        .unwrap_or_else(|| "No album info".to_string());
                    ui.label(egui::RichText::new(album_label).color(egui::Color32::WHITE).size(16.0));

                    if self.editing_album {
                        egui::Window::new("Edit Album").show(ctx, |ui| {
                            ui.text_edit_singleline(&mut self.edited_album);
                            if ui.button("Save").clicked() {
                                if let Some(path) = &self.selected_file {
                                    let album = self.edited_album.clone();
                                    if let Err(e) = app.set_album(path, album) {
                                        self.alert_message = format!("Failed to save album: {}", e);
                                    } else {
                                        self.editing_album = false;
                                    }
                                }
                            }
                            if ui.button("Cancel").clicked() {
                                self.editing_album = false;
                            }
                        });
                    }

                    ui.add_space(10.0);

                    // genre
                    ui.horizontal(|ui| {
                        ui.heading("Genre:");
                        if ui.button("edit").clicked() {
                            self.editing_genre = true;
                        }
                    });

                    let genre_label = self
                        .selected_file
                        .as_ref()
                        .and_then(|path| app.get_genre(path).ok().flatten())
                        .unwrap_or_else(|| "No genre info".to_string());
                    ui.label(egui::RichText::new(genre_label).color(egui::Color32::WHITE).size(16.0));

                    if self.editing_genre {
                        egui::Window::new("Edit Genre").show(ctx, |ui| {
                            ui.text_edit_singleline(&mut self.edited_genre);
                            if ui.button("Save").clicked() {
                                if let Some(path) = &self.selected_file {
                                    let genre = self.edited_genre.clone();
                                    if let Err(e) = app.set_genre(path, genre) {
                                        self.alert_message = format!("Failed to save genre: {}", e);
                                    } else {
                                        self.editing_genre = false;
                                    }
                                }
                            }
                            if ui.button("Cancel").clicked() {
                                self.editing_genre = false;
                            }
                        });
                    }

                    ui.add_space(5.0);

                    // quality
                    ui.vertical(|ui| {
                        ui.add_space(5.0);
                        ui.centered_and_justified(|ui| {
                            ui.columns(2, |columns| {
                                if let Some(path) = &self.selected_file {
                                    columns[0].heading(egui::RichText::new("Bitrate:").size(15.0));
                                    if let Ok(Some(b)) = MyApp::get_bitrate(path) {
                                        columns[0].label(egui::RichText::new(format!("{} kbps", b / 1000)).size(16.0).color(egui::Color32::WHITE));
                                    } else {
                                        columns[0].label("Unknown bitrate");
                                    }

                                    columns[0].add_space(5.0);

                                    columns[1].heading(egui::RichText::new("Sample Rate:").size(15.0));
                                    if let Ok(Some(sr)) = MyApp::get_sample_rate(path) {
                                        columns[1].label(egui::RichText::new(format!("{} kHz", sr / 1000)).size(16.0).color(egui::Color32::WHITE));
                                    } else {
                                        columns[1].label("Unknown sample rate");
                                    }
                                    
                                    columns[1].add_space(5.0);

                                    columns[0].heading(egui::RichText::new("Duration:").size(15.0));
                                    if let Ok(Some(s)) = MyApp::get_duration_seconds(path) {
                                        columns[0].label(egui::RichText::new(format!("{} seconds", s.floor() as u64)).size(16.0).color(egui::Color32::WHITE));
                                    } else {
                                        columns[0].label("Unknown duration");
                                    }
                                }  
                            });
                        });
                    });
                });
                });
                

                ui.vertical(|ui| {
                    ui.set_width(right_width);
                    
                    // image
                    ui.horizontal(|ui| {
                        ui.heading("Artwork:");
                        if ui.button("edit").clicked() {
                            if let Err(err) = self.pick_and_set_album_art(ctx) {
                                self.alert_message = format!("Failed to pick/set album art: {}", err);
                            }
                        }
                        if  self.selected_album_art.is_some() && ui.button("save image").clicked() {
                            if let Some(path) = &self.selected_file {
                                if let Err(err) = app.save_album_art(path) {
                                    self.alert_message = format!("Failed to save album art: {}", err);
                                }
                            }
                        }
                    });

                    if !self.album_art_ready { // not ready yet
                        ui.label("Loading...");
                    } else if let Some(tex) = &self.selected_album_art {
                        ui.image((*tex, egui::Vec2::splat(300.0)));
                    } else {
                        ui.label("No album art");
                    }
                });
            });
            } else {
               let available = ui.available_size();
                ui.allocate_ui(available, |ui| {
                    ui.with_layout(egui::Layout::centered_and_justified(egui::Direction::TopDown), |ui| {
                        ui.heading(egui::RichText::new("Add or drag a file!").size(30.0).color(egui::Color32::WHITE));
                    });
                });
            }
        });    
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut options = NativeOptions::default();
    options.viewport.resizable = Some(false);
    options.viewport.inner_size = Some(egui::vec2(800.0, 400.0));
    Ok(eframe::run_native(
        "Metadata Editor",
        options,
        Box::new(|_cc| Ok(Box::new(MyApp::default()))),
    )?)
}
