use super::*;

#[test]
fn png_original_prepares_native_dib_without_changing_pixels() {
    let original = image::RgbaImage::from_pixel(2, 3, image::Rgba([20, 70, 130, 255]));
    let mut encoded = std::io::Cursor::new(Vec::new());
    image::DynamicImage::ImageRgba8(original.clone())
        .write_to(&mut encoded, image::ImageOutputFormat::Png)
        .unwrap();
    let png = encoded.into_inner();
    let prepared = PreparedClipboard::new(&[representation("image", &png)]).unwrap();
    assert_eq!(prepared.0.len(), 2, "retain PNG and offer a native DIB");
    assert_eq!(prepared.0[0].0, register_format("PNG").unwrap());
    assert_eq!(prepared.0[1].0, 8);
    let dib = image_to_dib(&png).unwrap();
    let bmp = dib_to_bmp(&dib).unwrap();
    assert_eq!(image::load_from_memory(&bmp).unwrap().to_rgba8(), original);
    assert!(inline_image_fingerprint(&png).is_some());
}
fn representation(format: &str, bytes: &[u8]) -> ClipboardRepresentation {
    ClipboardRepresentation {
        format: format.into(),
        mime_type: "test/synthetic".into(),
        bytes: bytes.to_vec(),
    }
}
#[test]
fn allocation_is_freed_on_lock_and_publish_failures() {
    let memory = OwnedGlobal(unsafe { GlobalAlloc(GMEM_MOVEABLE, 16) }.unwrap());
    let handle = memory.0;
    assert!(memory.fill_with(b"test", |_| std::ptr::null_mut()).is_err());
    assert_eq!(unsafe { GlobalSize(handle) }, 0);
    let memory = OwnedGlobal::new(b"synthetic memory").unwrap();
    let handle = memory.0;
    assert!(memory
        .transfer_with(|_| Err(PlatformError("injected publish failure".into())))
        .is_err());
    assert_eq!(unsafe { GlobalSize(handle) }, 0);
}
#[test]
fn successful_transfer_relinquishes_application_ownership() {
    let memory = OwnedGlobal::new(b"synthetic memory").unwrap();
    let handle = memory.0;
    memory.transfer_with(|_| Ok(())).unwrap();
    assert!(unsafe { GlobalSize(handle) } >= 16);
    // A simulated recipient, rather than the application guard, owns cleanup now.
    let _ = unsafe { windows::Win32::Foundation::GlobalFree(Some(handle)) };
}
#[test]
fn invalid_formats_and_empty_batches_are_rejected_during_preparation() {
    assert!(PreparedClipboard::new(&[]).is_err());
    assert!(PreparedClipboard::new(&[representation("unsupported", b"x")]).is_err());
    assert!(PreparedClipboard::new(&[
        representation("text", b"valid"),
        representation("image", b"invalid")
    ])
    .is_err());
    assert!(PreparedClipboard::new(&[representation("text", b"bad\0text")]).is_err());
    assert!(PreparedClipboard::new(&[representation("text", &[0xff])]).is_err());
    assert!(PreparedClipboard::new(&[representation("files", b"")]).is_err());
    assert!(PreparedClipboard::new(&[representation("files", b"a\0b")]).is_err());
}
#[test]
fn invalid_owner_is_rejected_before_opening_or_clearing_clipboard() {
    assert!(ClipboardGuard::open_for_write(HWND::default()).is_err());
    let prepared = PreparedClipboard::new(&[representation("text", b"synthetic")]).unwrap();
    assert!(prepared.publish(HWND::default()).is_err());
}
#[test]
fn preparation_retains_unicode_and_each_original_format() {
    let prepared = PreparedClipboard::new(&[
        representation("text", "中文🙂".as_bytes()),
        representation("html", b"<b>original</b>"),
        representation("rtf", b"{\\rtf1 original}"),
        representation("files", b"C:\\synthetic.txt"),
    ])
    .unwrap();
    assert_eq!(prepared.0.len(), 4);
    assert_eq!(
        ClipboardGuard::unicode_bytes("中文🙂"),
        [0x2d, 0x4e, 0x87, 0x65, 0x3d, 0xd8, 0x42, 0xde, 0, 0]
    );
}

#[test]
#[ignore = "requires explicit acceptance authorization and an in-memory clipboard backup"]
fn authorized_clipboard_roundtrip_and_capture_exclusions() {
    assert_eq!(std::env::var("ECHO_WINDOWS_ACCEPTANCE").as_deref(), Ok("1"));
    assert_eq!(
        std::env::var("ECHO_CLIPBOARD_BACKUP_READY").as_deref(),
        Ok("1")
    );
    let platform = WindowsPlatform::new();
    let policy = CapturePolicy::default();
    let owner = HWND(platform.clipboard_window.load(Ordering::Acquire) as *mut c_void);
    let mut dib = vec![0_u8; 44];
    dib[0..4].copy_from_slice(&40_u32.to_le_bytes());
    dib[4..8].copy_from_slice(&1_i32.to_le_bytes());
    dib[8..12].copy_from_slice(&1_i32.to_le_bytes());
    dib[12..14].copy_from_slice(&1_u16.to_le_bytes());
    dib[14..16].copy_from_slice(&32_u16.to_le_bytes());
    dib[20..24].copy_from_slice(&4_u32.to_le_bytes());
    dib[40..44].copy_from_slice(&[20, 40, 60, 255]);
    let image = dib_to_bmp(&dib).expect("valid synthetic BMP");
    let original = vec![
        representation("text", "echo-audit-clipboard 中文🙂".as_bytes()),
        representation("html", b"<b>echo-audit-original</b>"),
        representation("rtf", b"{\\rtf1 echo-audit-original}"),
        representation("image", &image),
        representation(
            "files",
            b"C:\\EchoSynthetic\\one.txt\nC:\\EchoSynthetic\\two.txt",
        ),
    ];
    platform.write_clipboard(&original).unwrap();
    assert_eq!(unsafe { GetClipboardOwner() }.unwrap(), owner);
    let actual = platform.read_clipboard(&policy).unwrap().unwrap();
    for expected in &original {
        let actual = actual
            .representations
            .iter()
            .find(|r| r.format == expected.format)
            .unwrap();
        if expected.format == "image" {
            let offset = u32::from_le_bytes(actual.bytes[10..14].try_into().unwrap()) as usize;
            let header = u32::from_le_bytes(actual.bytes[14..18].try_into().unwrap());
            assert_eq!(&actual.bytes[18..26], &expected.bytes[18..26]);
            assert_eq!(&actual.bytes[offset..offset + 3], &[20, 40, 60]);
            let _guard = ClipboardGuard::open().unwrap();
            let retained = ClipboardGuard::read_global(8).unwrap();
            assert!(
                retained.starts_with(&dib),
                "original DIB pixels and header were changed"
            );
            println!("SYNTHETIC_BMP: written_header=40 read_header={header} written_bytes={} read_bytes={}", expected.bytes.len(), actual.bytes.len());
        } else {
            assert!(
                actual.bytes == expected.bytes,
                "original format did not roundtrip: {}",
                expected.format
            );
        }
    }
    // Invalid preparation must not clear an existing valid clipboard.
    assert!(platform
        .write_clipboard(&[representation("image", b"bad BMP")])
        .is_err());
    assert!(platform.read_clipboard(&policy).unwrap().is_some());
    for (name, value, capture) in [
        ("ExcludeClipboardContentFromMonitorProcessing", 1_u32, false),
        ("CanIncludeInClipboardHistory", 0, false),
        ("CanIncludeInClipboardHistory", 1, true),
        ("CanIncludeInClipboardHistory", 2, false),
        ("CanUploadToCloudClipboard", 0, true),
    ] {
        platform.write_clipboard(&original[..1]).unwrap();
        {
            let _guard = ClipboardGuard::open_for_write(owner).unwrap();
            OwnedGlobal::new(&value.to_le_bytes())
                .unwrap()
                .publish(register_format(name).unwrap())
                .unwrap();
        }
        assert_eq!(
            platform.read_clipboard(&policy).unwrap().is_some(),
            capture,
            "policy {name}"
        );
    }
    platform.write_clipboard(&original[..1]).unwrap();
    println!("CLIPBOARD_NATIVE: original text/html/rtf/image/files roundtrip, valid owner, invalid preparation preservation and five exclusion policies passed");
}
