use std::fmt::Write;

pub(crate) fn lowercase_hex(bytes: impl AsRef<[u8]>) -> String {
    let bytes = bytes.as_ref();
    let mut encoded = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        write!(encoded, "{byte:02x}").expect("writing to a String cannot fail");
    }
    encoded
}

#[cfg(test)]
mod tests {
    use super::lowercase_hex;

    #[test]
    fn encodes_bytes_as_lowercase_hex() {
        assert_eq!(lowercase_hex([0x00, 0x0f, 0xa5, 0xff]), "000fa5ff");
    }
}
