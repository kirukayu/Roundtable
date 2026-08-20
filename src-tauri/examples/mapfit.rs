//! Dumps the two tables that pin down the world-to-map transform.
//!
//! `WorldMapPieceParam` gives the map's own rectangles, in exactly the
//! coordinates a marker is stored in. `WorldMapPointParam` gives every named
//! point on the map in world coordinates. Fitting one onto the other is what
//! turns "mark Stormveil" into a pair of numbers.
//!
//!     cargo run --example mapfit -- <regulation.bin>

use roundtable_lib::formats::regulation::Regulation;

/// Field offsets, worked out from the paramdef field order and the way a
/// `dummy8` with a bit size shares a byte with what came before it.
mod piece {
    pub const LEFT: usize = 0x08;
    pub const RIGHT: usize = 0x0c;
    pub const TOP: usize = 0x10;
    pub const BOTTOM: usize = 0x14;
}

mod point {
    pub const AREA: usize = 0x20;
    pub const GRID_X: usize = 0x21;
    pub const GRID_Z: usize = 0x22;
    pub const POS_X: usize = 0x24;
    pub const POS_Y: usize = 0x28;
    pub const POS_Z: usize = 0x2c;
}

/// A place name knows which piece it belongs to, which is the pairing the fit
/// needs: this point must land inside that rectangle, not merely somewhere.
mod named {
    pub const PIECE: usize = 0x04;
    pub const TEXT: usize = 0x08;
    pub const AREA: usize = 0x10;
    pub const GRID_X: usize = 0x11;
    pub const GRID_Z: usize = 0x12;
    pub const POS_X: usize = 0x14;
    pub const POS_Z: usize = 0x1c;
}

/// The overworld's tiles are this many units across.
const TILE: f32 = 256.0;

fn main() {
    let Some(path) = std::env::args().nth(1) else {
        eprintln!("usage: cargo run --example mapfit -- <regulation.bin>");
        return;
    };

    let regulation = match Regulation::open(std::path::Path::new(&path)) {
        Ok(regulation) => regulation,
        Err(problem) => {
            eprintln!("{path}: {problem}");
            return;
        }
    };

    if let Some(pieces) = regulation.table("WorldMapPieceParam") {
        println!("# pieces\tid\tleft\tright\ttop\tbottom");
        let mut ids: Vec<i64> = pieces.ids().collect();
        ids.sort_unstable();
        for id in ids {
            let (Some(left), Some(right), Some(top), Some(bottom)) = (
                pieces.f32(id, piece::LEFT),
                pieces.f32(id, piece::RIGHT),
                pieces.f32(id, piece::TOP),
                pieces.f32(id, piece::BOTTOM),
            ) else {
                continue;
            };
            println!("piece\t{id}\t{left}\t{right}\t{top}\t{bottom}");
        }
    } else {
        println!("# no WorldMapPieceParam");
    }

    if let Some(points) = regulation.table("WorldMapPointParam") {
        println!("# points\tid\tarea\tgridX\tgridZ\tposX\tposY\tposZ\tglobalX\tglobalZ");
        let mut ids: Vec<i64> = points.ids().collect();
        ids.sort_unstable();
        for id in ids {
            let (Some(area), Some(grid_x), Some(grid_z), Some(x), Some(y), Some(z)) = (
                points.u8(id, point::AREA),
                points.u8(id, point::GRID_X),
                points.u8(id, point::GRID_Z),
                points.f32(id, point::POS_X),
                points.f32(id, point::POS_Y),
                points.f32(id, point::POS_Z),
            ) else {
                continue;
            };
            let global_x = f32::from(grid_x) * TILE + x;
            let global_z = f32::from(grid_z) * TILE + z;
            println!(
                "point\t{id}\t{area}\t{grid_x}\t{grid_z}\t{x}\t{y}\t{z}\t{global_x}\t{global_z}"
            );
        }
    } else {
        println!("# no WorldMapPointParam");
    }

    if let Some(places) = regulation.table("WorldMapPlaceNameParam") {
        println!("# places\tid\tpiece\ttext\tarea\tgridX\tgridZ\tglobalX\tglobalZ");
        let mut ids: Vec<i64> = places.ids().collect();
        ids.sort_unstable();
        for id in ids {
            let (Some(piece), Some(text), Some(area), Some(grid_x), Some(grid_z), Some(x), Some(z)) = (
                places.i32(id, named::PIECE),
                places.i32(id, named::TEXT),
                places.u8(id, named::AREA),
                places.u8(id, named::GRID_X),
                places.u8(id, named::GRID_Z),
                places.f32(id, named::POS_X),
                places.f32(id, named::POS_Z),
            ) else {
                continue;
            };
            println!(
                "place\t{id}\t{piece}\t{text}\t{area}\t{grid_x}\t{grid_z}\t{}\t{}",
                f32::from(grid_x) * TILE + x,
                f32::from(grid_z) * TILE + z
            );
        }
    } else {
        println!("# no WorldMapPlaceNameParam");
    }
}
