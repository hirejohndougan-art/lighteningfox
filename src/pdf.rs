//! Print and "Save as PDF": the page is laid out for paper, drawn page by page, and each
//! sheet becomes a JPEG image in a small hand-written PDF file (no PDF library needed).
//! The text isn't selectable in the PDF yet; it looks like the printed page.

use std::io::Write;

/// US Letter in PDF points (1/72 inch), with half-inch margins.
pub const PAGE_W_PT: f32 = 612.0;
pub const PAGE_H_PT: f32 = 792.0;
pub const MARGIN_PT: f32 = 36.0;

/// The printable area in CSS pixels (96 per inch): 7.5 × 10 inches.
pub const CONTENT_W_PX: f32 = (PAGE_W_PT - 2.0 * MARGIN_PT) / 72.0 * 96.0;
pub const CONTENT_H_PX: f32 = (PAGE_H_PT - 2.0 * MARGIN_PT) / 72.0 * 96.0;

/// Write `pages` (each an image of one sheet's printable area) as a PDF.
pub fn write(path: &std::path::Path, pages: &[image::RgbImage]) -> std::io::Result<u64> {
    let mut out: Vec<u8> = Vec::new();
    let mut offsets: Vec<usize> = Vec::new();
    out.extend_from_slice(b"%PDF-1.4\n%\xe2\xe3\xcf\xd3\n");
    let n = pages.len();
    // Objects: 1 catalog, 2 page tree, then per page: page, contents, image.
    let page_obj = |i: usize| 3 + i * 3;
    let obj = |out: &mut Vec<u8>, offsets: &mut Vec<usize>, body: &[u8]| {
        offsets.push(out.len());
        let id = offsets.len();
        out.extend_from_slice(format!("{id} 0 obj\n").as_bytes());
        out.extend_from_slice(body);
        out.extend_from_slice(b"\nendobj\n");
    };
    obj(&mut out, &mut offsets, b"<< /Type /Catalog /Pages 2 0 R >>");
    let kids: Vec<String> = (0..n).map(|i| format!("{} 0 R", page_obj(i))).collect();
    obj(&mut out, &mut offsets, format!("<< /Type /Pages /Kids [{}] /Count {n} >>", kids.join(" ")).as_bytes());
    let (cw, ch) = (PAGE_W_PT - 2.0 * MARGIN_PT, PAGE_H_PT - 2.0 * MARGIN_PT);
    for (i, img) in pages.iter().enumerate() {
        let (contents, image) = (page_obj(i) + 1, page_obj(i) + 2);
        obj(
            &mut out,
            &mut offsets,
            format!(
                "<< /Type /Page /Parent 2 0 R /MediaBox [0 0 {PAGE_W_PT} {PAGE_H_PT}] /Resources << /XObject << /Im{i} {image} 0 R >> >> /Contents {contents} 0 R >>"
            )
            .as_bytes(),
        );
        // The image fills the printable width; its height follows its proportions.
        let h_pt = cw * img.height() as f32 / img.width().max(1) as f32;
        let draw = format!("q {cw} 0 0 {h_pt} {MARGIN_PT} {} cm /Im{i} Do Q", MARGIN_PT + ch - h_pt);
        obj(&mut out, &mut offsets, format!("<< /Length {} >>\nstream\n{draw}\nendstream", draw.len()).as_bytes());
        let mut jpeg = Vec::new();
        image::codecs::jpeg::JpegEncoder::new_with_quality(&mut jpeg, 88).encode_image(img).map_err(std::io::Error::other)?;
        let mut body = format!(
            "<< /Type /XObject /Subtype /Image /Width {} /Height {} /ColorSpace /DeviceRGB /BitsPerComponent 8 /Filter /DCTDecode /Length {} >>\nstream\n",
            img.width(),
            img.height(),
            jpeg.len()
        )
        .into_bytes();
        body.extend_from_slice(&jpeg);
        body.extend_from_slice(b"\nendstream");
        obj(&mut out, &mut offsets, &body);
    }
    let xref = out.len();
    out.extend_from_slice(format!("xref\n0 {}\n0000000000 65535 f \n", offsets.len() + 1).as_bytes());
    for o in &offsets {
        out.extend_from_slice(format!("{o:010} 00000 n \n").as_bytes());
    }
    out.extend_from_slice(format!("trailer\n<< /Size {} /Root 1 0 R >>\nstartxref\n{xref}\n%%EOF\n", offsets.len() + 1).as_bytes());
    let mut f = std::fs::File::create(path)?;
    f.write_all(&out)?;
    Ok(out.len() as u64)
}

#[cfg(test)]
mod tests {
    #[test]
    fn writes_a_pdf_with_one_page_per_image() {
        let dir = std::env::temp_dir().join(format!("ironswift-pdf-{}", std::process::id()));
        let _ = std::fs::create_dir_all(&dir);
        let p = dir.join("t.pdf");
        let img = image::RgbImage::from_pixel(100, 120, image::Rgb([200, 30, 30]));
        let n = super::write(&p, &[img.clone(), img]).unwrap();
        let bytes = std::fs::read(&p).unwrap();
        assert_eq!(bytes.len() as u64, n);
        let text = String::from_utf8_lossy(&bytes);
        assert!(text.starts_with("%PDF-1.4"));
        assert!(text.contains("/Count 2"));
        assert!(text.trim_end().ends_with("%%EOF"));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
