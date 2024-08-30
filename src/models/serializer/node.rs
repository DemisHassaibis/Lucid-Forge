use super::CustomSerialize;
use crate::models::{
    cache_loader::NodeRegistry,
    lazy_load::{EagerLazyItemSet, FileIndex, LazyItemRef},
    types::{BytesToRead, FileOffset, HNSWLevel, MergedNode, PropState},
};
use arcshift::ArcShift;
use byteorder::{LittleEndian, ReadBytesExt, WriteBytesExt};
use std::collections::HashSet;
use std::{
    io::{Read, Seek, SeekFrom, Write},
    sync::Arc,
};

impl CustomSerialize for MergedNode {
    fn serialize<W: Write + Seek>(&self, writer: &mut W) -> std::io::Result<u32> {
        let start_offset = writer.stream_position()? as u32;

        // Serialize basic fields
        writer.write_u8(self.hnsw_level.0)?;

        // Serialize prop
        let mut prop = self.prop.clone();
        let prop_state = prop.get();
        match &*prop_state {
            PropState::Ready(node_prop) => {
                if let Some((FileOffset(offset), BytesToRead(length))) = node_prop.location {
                    writer.write_u32::<LittleEndian>(offset)?;
                    writer.write_u32::<LittleEndian>(length)?;
                } else {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::InvalidData,
                        "Ready PropState with no location",
                    ));
                }
            }
            PropState::Pending((FileOffset(offset), BytesToRead(length))) => {
                writer.write_u32::<LittleEndian>(*offset)?;
                writer.write_u32::<LittleEndian>(*length)?;
            }
        }

        // Create and write indicator byte
        let mut indicator: u8 = 0;
        let parent_present = self.parent.is_valid();
        let child_present = self.child.is_valid();
        if parent_present {
            indicator |= 0b00000001;
        }
        if child_present {
            indicator |= 0b00000010;
        }
        writer.write_u8(indicator)?;

        // Write placeholders only for present parent and child
        let parent_placeholder = if parent_present {
            let pos = writer.stream_position()? as u32;
            writer.write_u32::<LittleEndian>(0)?;
            writer.write_u32::<LittleEndian>(0)?;
            Some(pos)
        } else {
            None
        };

        let child_placeholder = if child_present {
            let pos = writer.stream_position()? as u32;
            writer.write_u32::<LittleEndian>(0)?;
            writer.write_u32::<LittleEndian>(0)?;
            Some(pos)
        } else {
            None
        };

        // Write placeholders for neighbors and versions
        let neighbors_placeholder = writer.stream_position()? as u32;
        writer.write_u32::<LittleEndian>(u32::MAX)?;

        // Serialize parent if present
        let parent_offset = if parent_present {
            Some(self.parent.serialize(writer)?)
        } else {
            None
        };

        // Serialize child if present
        let child_offset = if child_present {
            Some(self.child.serialize(writer)?)
        } else {
            None
        };

        // Serialize neighbors
        let neighbors_offset = self.neighbors.serialize(writer)?;

        // Update placeholders
        let end_pos = writer.stream_position()?;

        if let (Some(placeholder), Some(offset)) = (parent_placeholder, parent_offset) {
            writer.seek(SeekFrom::Start(placeholder as u64))?;
            writer.write_u32::<LittleEndian>(offset)?;
            writer.write_u32::<LittleEndian>(*self.parent.get_current_version())?;
        }

        if let (Some(placeholder), Some(offset)) = (child_placeholder, child_offset) {
            writer.seek(SeekFrom::Start(placeholder as u64))?;
            writer.write_u32::<LittleEndian>(offset)?;
            writer.write_u32::<LittleEndian>(*self.child.get_current_version())?;
        }

        writer.seek(SeekFrom::Start(neighbors_placeholder as u64))?;
        writer.write_u32::<LittleEndian>(neighbors_offset)?;

        // Return to the end of the serialized data
        writer.seek(SeekFrom::Start(end_pos))?;

        Ok(start_offset)
    }

    fn deserialize<R: Read + Seek>(
        reader: &mut R,
        file_index: FileIndex,
        cache: Arc<NodeRegistry<R>>,
        max_loads: u16,
        skipm: &mut HashSet<u64>,
    ) -> std::io::Result<Self> {
        match file_index {
            FileIndex::Invalid => Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Cannot deserialize MergedNode with an invalid FileIndex",
            )),
            FileIndex::Valid {
                offset: FileOffset(offset),
                version,
            } => {
                reader.seek(SeekFrom::Start(offset as u64))?;
                // Read basic fields
                let hnsw_level = HNSWLevel(reader.read_u8()?);
                // Read prop
                let prop_offset = FileOffset(reader.read_u32::<LittleEndian>()?);
                let prop_length = BytesToRead(reader.read_u32::<LittleEndian>()?);
                let prop = PropState::Pending((prop_offset, prop_length));
                // Read indicator byte
                let indicator = reader.read_u8()?;
                let parent_present = indicator & 0b00000001 != 0;
                let child_present = indicator & 0b00000010 != 0;
                // Read offsets
                let mut parent_offset_and_version = None;
                let mut child_offset_and_version = None;
                if parent_present {
                    parent_offset_and_version = Some((
                        reader.read_u32::<LittleEndian>()?,
                        reader.read_u32::<LittleEndian>()?.into(),
                    ));
                }
                if child_present {
                    child_offset_and_version = Some((
                        reader.read_u32::<LittleEndian>()?,
                        reader.read_u32::<LittleEndian>()?.into(),
                    ));
                }
                let neighbors_offset = reader.read_u32::<LittleEndian>()?;
                // Deserialize parent
                let parent = if let Some((offset, version)) = parent_offset_and_version {
                    LazyItemRef::deserialize(
                        reader,
                        FileIndex::Valid {
                            offset: FileOffset(offset),
                            version,
                        },
                        cache.clone(),
                        max_loads,
                        skipm,
                    )?
                } else {
                    LazyItemRef::new_invalid()
                };
                // Deserialize child
                let child = if let Some((offset, version)) = child_offset_and_version {
                    LazyItemRef::deserialize(
                        reader,
                        FileIndex::Valid {
                            offset: FileOffset(offset),
                            version,
                        },
                        cache.clone(),
                        max_loads,
                        skipm,
                    )?
                } else {
                    LazyItemRef::new_invalid()
                };
                // Deserialize neighbors
                let neighbors = EagerLazyItemSet::deserialize(
                    reader,
                    FileIndex::Valid {
                        offset: FileOffset(neighbors_offset),
                        version,
                    },
                    cache.clone(),
                    max_loads,
                    skipm,
                )?;

                Ok(MergedNode {
                    hnsw_level,
                    prop: ArcShift::new(prop),
                    neighbors,
                    parent,
                    child,
                })
            }
        }
    }
}
