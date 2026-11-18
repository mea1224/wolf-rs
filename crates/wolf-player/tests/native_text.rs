#[path = "../src/native_text.rs"]
mod native_text;
#[path = "../src/native_text_win.rs"]
mod native_text_win;

#[cfg(windows)]
#[test]
fn font_callback_renders_glyph_coverage_and_measures_advance() {
    let text = native_text_win::raster("OK", 22, "MS Gothic").unwrap();
    assert!(text.advance > 0);
    assert!(text.line_height >= 20);
    assert!(text.image.bytes().chunks_exact(4).any(|p| p[3] > 0));
    assert!(text.image.bytes().chunks_exact(4).any(|p| p[3] == 0));
    assert!(native_text_win::raster("a\0b", 22, "MS Gothic").is_err());
}
use native_text::{Inline, RasterText};
use std::sync::Arc;
use wolf_media::Image;

#[test]
fn dialog_outline_controls_reach_raster_without_changing_logical_advance() {
    let mut seen=Vec::new();
    let text=native_text::layout_with_outline("\\wE[0]\\cE[7]A\\wE[99]\\cE[-8]B",28,0,32,
        |_|Err("unexpected image".into()),
        |text,size,font,color,width,edge| {
            seen.push((text.to_owned(),size,font,color,width,edge));
            Ok(RasterText {image:Arc::new(Image::new(8,10,vec![255;320])?),advance:8,line_height:10})
        }).unwrap();
    assert_eq!(seen,vec![("A".into(),28,0,0,1,7),("B".into(),28,0,0,5,-1)]);
    assert_eq!((text.width,text.height),(16,10));
}

#[cfg(windows)]
#[test]
fn outline_changes_coverage_and_retains_font_measurement() {
    let plain=native_text_win::raster_color("Alice",32,"MS Gothic",[230,240,250]).unwrap();
    let edge=native_text_win::raster_outline("Alice",32,"MS Gothic",[230,240,250],2,[80,20,100]).unwrap();
    assert_eq!((plain.advance,plain.line_height),(edge.advance,edge.line_height));
    assert_eq!(plain.image.dimensions(),edge.image.dimensions());
    assert!(plain.image.bytes().chunks_exact(4).zip(edge.image.bytes().chunks_exact(4)).any(|(a,b)|b[3]>a[3]));
    assert!(edge.image.bytes().chunks_exact(4).any(|p|p[..3]==[80,20,100] && p[3]>0));
}

#[test]
fn saved_native_text_packets_preserve_text_and_reject_incomplete_or_unrendered_style() {
    let mut source = vec![1, 5, 3, 14, 36, 1, 1, 6, 2, 14, 14, 14, 1];
    source.extend_from_slice("逃げないといけない気がする。".as_bytes());
    assert_eq!(native_text::decode_saved_text(&source).unwrap(), "\\f[22]\\c[0]逃げないといけない気がする。");
    let cleared = [1,3,7,124,32,1,2,32,1,5,35,14,17,1,1,5,33,14,14,22,1,5,3,14,29,1,1,6,2,14,14,14,1];
    assert_eq!(native_text::decode_saved_text(&cleared).unwrap(), "");
    let mut styled = cleared.to_vec();
    styled.push(b'X');
    let decoded = native_text::decode_saved_text(&styled).unwrap();
    assert!(decoded.contains("\\wE[3]\\cE[7]"));
    assert!(decoded.ends_with('X'));
    assert!(native_text::decode_saved_text(&[1,5,3,14]).is_err());
    assert!(native_text::decode_saved_text(&[1,1]).is_err());
    let absolute = [1,5,0x1d,92,22,1,b'X'];
    assert_eq!(native_text::decode_saved_text(&absolute).unwrap(), "\\ax[0]X");
}

#[test]
fn dialog_style_line_spacing_and_alignment_use_measured_rows() {
    let mut styles = Vec::new();
    let layout = native_text::layout_with_style(
        "\\space[12]<C>\\font[2]\\f[10]\\c[3]LONG\r\nOK",
        28,
        |_| Err("no image expected".into()),
        |text,size,font,color| {
            styles.push((text.to_owned(),size,font,color));
            let width = text.len() as u32 * 5;
            Ok(RasterText { image:Arc::new(Image::new(width as usize,size as usize,vec![255;width as usize*size as usize*4])?),advance:width as i32,line_height:size })
        },
    ).unwrap();
    assert_eq!(styles,[("LONG".into(),10,2,3),("OK".into(),10,2,3)]);
    assert_eq!((layout.width,layout.height),(20,32));
    assert_eq!((layout.images[0].x,layout.images[0].y),(0,0));
    assert_eq!((layout.images[1].x,layout.images[1].y),(5,22));
}

#[test]
fn absolute_x_replaces_cursor_and_alignment_does_not_print_markup() {
    let layout = native_text::layout("A\\ax[50]B",10,|_|Err("no image".into()),|_,size|Ok(RasterText {
        image:Arc::new(Image::new(5,size as usize,vec![255;5*size as usize*4])?),advance:5,line_height:size,
    })).unwrap();
    assert_eq!(layout.width,55);
    assert_eq!(layout.images[1].x,50);
    assert_eq!(native_text::parse("<C>a<R>b<L>c").unwrap(),vec![Inline::Align(1),Inline::Text("a".into()),Inline::Align(2),Inline::Text("b".into()),Inline::Align(0),Inline::Text("c".into())]);
}

#[test]
fn common89_controls_preserve_icon_path_and_clamp_native_arguments() {
    assert_eq!(
        native_text::parse(r"\>\f[22]\my[4]\img[BasicData\icon90.png]\my[-4]\mx[10]OK").unwrap(),
        vec![
            Inline::Instant,
            Inline::FontSize(22),
            Inline::OffsetY(4),
            Inline::Image(r"BasicData\icon90.png".into()),
            Inline::OffsetY(-4),
            Inline::OffsetX(10),
            Inline::Text("OK".into())
        ]
    );
    assert_eq!(
        native_text::parse(r"\f[-2]\f[1000]\mx[-9999]\my[9999]").unwrap(),
        vec![
            Inline::FontSize(2),
            Inline::FontSize(999),
            Inline::OffsetX(-5000),
            Inline::OffsetY(5000)
        ]
    );
    assert!(native_text::parse(r"\img[unfinished").is_err());
    assert!(native_text::parse(r"\F[22]").is_err());
}

#[test]
fn character_compression_applies_per_glyph_and_saved_packet_round_trips() {
    let mut seen = Vec::new();
    let layout = native_text::layout(r"\-[2]ABC", 10, |_| Err("no image".into()), |text, size| {
        seen.push(text.to_owned());
        Ok(RasterText {
            image: Arc::new(Image::new(5, size as usize, vec![255; 5 * size as usize * 4])?),
            advance: 5,
            line_height: size,
        })
    }).unwrap();
    assert_eq!(seen, ["A", "B", "C"]);
    assert_eq!(layout.width, 9);

    let packet = [1, 3, 7, 122, b'X'];
    assert_eq!(native_text::decode_saved_text(&packet).unwrap(), r"\-[2]X");
}

#[test]
fn inline_resource_and_text_positions_come_from_control_stream() {
    let icon = Arc::new(Image::new(8, 8, vec![255; 8 * 8 * 4]).unwrap());
    let layout = native_text::layout(
        r"\>\f[22]\my[4]\img[icon.png]\my[-4]\mx[10]OK",
        28,
        |path| {
            assert_eq!(path, "icon.png");
            Ok(icon.clone())
        },
        |text, size| {
            assert_eq!((text, size), ("OK", 22));
            Ok(RasterText {
                image: Arc::new(Image::new(25, 22, vec![255; 25 * 22 * 4]).unwrap()),
                advance: 25,
                line_height: 22,
            })
        },
    )
    .unwrap();
    assert_eq!((layout.width, layout.height), (43, 22));
    assert_eq!((layout.images[0].x, layout.images[0].y), (0, 18));
    assert_eq!((layout.images[1].x, layout.images[1].y), (18, 0));
    assert_eq!(layout.image().unwrap().dimensions(), (43, 22));
}
