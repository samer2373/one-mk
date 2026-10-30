use std::collections::BTreeSet;

fn word(bytes: &[u8], offset: usize, size: usize) -> Result<u64, String> {
    let field = bytes.get(offset..offset.checked_add(size).ok_or("ELF offset overflow")?).ok_or("truncated ELF")?;
    let mut value = [0; 8];
    value[..size].copy_from_slice(field);
    Ok(u64::from_le_bytes(value))
}

pub(crate) fn validate(bytes: &[u8], names: &BTreeSet<&str>) -> Result<(), String> {
    if !bytes.starts_with(b"\x7fELF") { return Ok(()); }
    if bytes.get(4..7) != Some(&[2, 1, 1]) { return Err("runtime requires ELF64 little endian version 1".into()); }
    let offset = usize::try_from(word(bytes, 32, 8)?).map_err(|_| "ELF offset overflow")?;
    let stride = word(bytes, 54, 2)? as usize;
    let count = word(bytes, 56, 2)? as usize;
    if stride != 56 || count > 1024 { return Err("invalid ELF program header table".into()); }
    let mut loads = Vec::new();
    let mut dynamic = None;
    for index in 0..count {
        let header = offset.checked_add(index * stride).ok_or("ELF header overflow")?;
        let kind = word(bytes, header, 4)?;
        let file_offset = word(bytes, header + 8, 8)?;
        let address = word(bytes, header + 16, 8)?;
        let size = word(bytes, header + 32, 8)?;
        let end = file_offset.checked_add(size).ok_or("ELF segment overflow")?;
        if end > bytes.len() as u64 { return Err("truncated ELF segment".into()); }
        if kind == 1 { loads.push((address, file_offset, size)); }
        if kind == 2 && dynamic.replace((file_offset as usize, size as usize)).is_some() { return Err("duplicate ELF dynamic segment".into()); }
    }
    let Some((offset, size)) = dynamic else { return Ok(()); };
    if size % 16 != 0 || size > 1024 * 1024 { return Err("ELF dynamic table limit".into()); }
    let mut needed = Vec::new();
    let mut strings = None;
    let mut string_size = None;
    let mut terminated = false;
    for entry in (offset..offset + size).step_by(16) {
        let tag = word(bytes, entry, 8)?;
        let value = word(bytes, entry + 8, 8)?;
        match tag {
            0 => { terminated = true; break; }
            1 => needed.push(value),
            5 => strings = Some(value),
            10 => string_size = Some(value),
            15 | 29 | 0x7fff_fffd | 0x7fff_ffff => return Err("unreviewed ELF loader search path/filter".into()),
            _ => {}
        }
    }
    if !terminated { return Err("unterminated ELF dynamic table".into()); }
    if needed.is_empty() { return Ok(()); }
    let address = strings.ok_or("ELF missing dynamic strings")?;
    let length = string_size.ok_or("ELF missing dynamic string size")?;
    let (start, file_offset, _) = loads.iter().find(|(start, _, size)| address >= *start && address.checked_add(length).zip(start.checked_add(*size)).is_some_and(|(end, ceiling)| end <= ceiling)).ok_or("ELF strings outside load segment")?;
    let offset = file_offset.checked_add(address - start).ok_or("ELF strings overflow")?;
    let table = bytes.get(offset as usize..offset.checked_add(length).ok_or("ELF strings overflow")? as usize).ok_or("truncated ELF strings")?;
    for index in needed {
        let name = table.get(usize::try_from(index).map_err(|_| "ELF string offset overflow")?..).ok_or("ELF string offset")?;
        let end = name.iter().position(|byte| *byte == 0).ok_or("unterminated ELF dependency")?;
        let name = std::str::from_utf8(&name[..end]).map_err(|_| "invalid ELF dependency")?;
        if name.contains('/') || !names.contains(name) { return Err(format!("forbidden/unbundled dynamic edge: {name}")); }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dynamic_edges_and_loader_paths_are_checked() {
        let mut bytes = vec![0; 256];
        bytes[..7].copy_from_slice(b"\x7fELF\x02\x01\x01");
        bytes[32..40].copy_from_slice(&64u64.to_le_bytes());
        bytes[54..58].copy_from_slice(&[56, 0, 2, 0]);
        bytes[64..68].copy_from_slice(&1u32.to_le_bytes());
        bytes[96..104].copy_from_slice(&256u64.to_le_bytes());
        bytes[120..124].copy_from_slice(&2u32.to_le_bytes());
        bytes[128..136].copy_from_slice(&176u64.to_le_bytes());
        bytes[152..160].copy_from_slice(&64u64.to_le_bytes());
        for (offset, tag, value) in [(176, 1u64, 0u64), (192, 5, 240), (208, 10, 16)] {
            bytes[offset..offset + 8].copy_from_slice(&tag.to_le_bytes());
            bytes[offset + 8..offset + 16].copy_from_slice(&value.to_le_bytes());
        }
        bytes[240..255].copy_from_slice(b"libfixture.so\0\0");
        assert!(validate(&bytes, &BTreeSet::new()).unwrap_err().contains("unbundled dynamic edge"));
        let names = BTreeSet::from(["libfixture.so"]);
        assert!(validate(&bytes, &names).is_ok());
        bytes[176..184].copy_from_slice(&29u64.to_le_bytes());
        assert!(validate(&bytes, &names).unwrap_err().contains("loader search path"));
    }
}
