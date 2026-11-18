use anyhow::{Result, bail};
use image::RgbaImage;
use windows::{
    Globalization::Language,
    Graphics::Imaging::{BitmapAlphaMode, BitmapPixelFormat, SoftwareBitmap},
    Media::Ocr::OcrEngine,
    Storage::Streams::DataWriter,
    Win32::System::WinRT::{RO_INIT_MULTITHREADED, RoInitialize, RoUninitialize},
    core::HSTRING,
};

#[derive(Clone)]
pub struct Word {
    pub text: String,
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
    pub line: usize,
}

pub fn recognize(image: RgbaImage, language: String) -> Result<Vec<Word>> {
    unsafe {
        RoInitialize(RO_INIT_MULTITHREADED)?;
    }
    struct Apartment;
    impl Drop for Apartment {
        fn drop(&mut self) {
            unsafe {
                RoUninitialize();
            }
        }
    }
    let _apartment = Apartment;
    let tags = match language.as_str() {
        "ru" => vec!["ru"],
        "en" => vec!["en-US"],
        _ => vec!["ru", "en-US"],
    };
    let mut engines = Vec::new();
    for tag in tags {
        let lang = Language::CreateLanguage(&HSTRING::from(tag))?;
        if OcrEngine::IsLanguageSupported(&lang)? {
            engines.push(OcrEngine::TryCreateFromLanguage(&lang)?);
        }
    }
    if engines.is_empty() {
        if let Ok(engine) = OcrEngine::TryCreateFromUserProfileLanguages() {
            engines.push(engine);
        }
    }
    if engines.is_empty() {
        bail!("Установите языковой пакет распознавания текста Windows.");
    }
    let side = image.width().max(image.height()) as f32;
    let scale = (if side < 1200.0 { 2.0_f32 } else { 1.0 })
        .min(OcrEngine::MaxImageDimension()? as f32 / side);
    let scaled = image::imageops::resize(
        &image,
        (image.width() as f32 * scale).round().max(1.0) as u32,
        (image.height() as f32 * scale).round().max(1.0) as u32,
        image::imageops::FilterType::Lanczos3,
    );
    let mut bytes = scaled.as_raw().clone();
    for pixel in bytes.chunks_exact_mut(4) {
        pixel.swap(0, 2);
        pixel[3] = 255;
    }
    let writer = DataWriter::new()?;
    writer.WriteBytes(&bytes)?;
    let bitmap = SoftwareBitmap::CreateCopyWithAlphaFromBuffer(
        &writer.DetachBuffer()?,
        BitmapPixelFormat::Bgra8,
        scaled.width() as i32,
        scaled.height() as i32,
        BitmapAlphaMode::Premultiplied,
    )?;
    let mut best = Vec::new();
    let mut best_score = 0;
    for engine in engines {
        let result = engine.RecognizeAsync(&bitmap)?.join()?;
        let score = result
            .Text()?
            .to_string()
            .chars()
            .filter(|c| c.is_alphanumeric())
            .count();
        if score <= best_score {
            continue;
        }
        best_score = score;
        best.clear();
        for (li, line) in result.Lines()?.into_iter().enumerate() {
            for word in line.Words()? {
                let r = word.BoundingRect()?;
                best.push(Word {
                    text: word.Text()?.to_string(),
                    x: r.X / scale,
                    y: r.Y / scale,
                    w: r.Width / scale,
                    h: r.Height / scale,
                    line: li,
                });
            }
        }
    }
    bitmap.Close()?;
    Ok(best)
}
