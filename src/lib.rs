use std::{
    collections::{BTreeMap, VecDeque},
    io::Cursor,
    str::Lines,
};
use zip::ZipArchive;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Database {
    Cities500,
    Cities1000,
    Cities5000,
    Cities15000,
}

impl Database {
    const fn name(self) -> &'static str {
        match self {
            Database::Cities500 => "cities500",
            Database::Cities1000 => "cities1000",
            Database::Cities5000 => "cities5000",
            Database::Cities15000 => "cities15000",
        }
    }

    const fn city_count(self) -> usize {
        match self {
            Database::Cities500 => 500,
            Database::Cities1000 => 1000,
            Database::Cities5000 => 5000,
            Database::Cities15000 => 15000,
        }
    }
}

impl Database {
    #[cfg(feature = "static")]
    fn zip_bytes(self) -> &'static [u8] {
        match self {
            Database::Cities500 => embedded::CITIES500,
            Database::Cities1000 => embedded::CITIES1000,
            Database::Cities5000 => embedded::CITIES5000,
            Database::Cities15000 => embedded::CITIES15000,
        }
    }

    #[cfg(feature = "static")]
    pub fn retrieve(self) -> Result<LoadedDatabase, error::DatabaseRetrieveError> {
        use std::io::Read;

        let mut zip = ZipArchive::new(Cursor::new(self.zip_bytes()))
            .map_err(error::DatabaseRetrieveError::Zip)?;
        let mut file = zip
            .by_name(&format!("{}.txt", self.name()))
            .map_err(error::DatabaseRetrieveError::Zip)?;

        let mut out = String::with_capacity(file.size() as usize);
        file.read_to_string(&mut out)
            .map_err(error::DatabaseRetrieveError::Io)?;

        Ok(LoadedDatabase::new(out, self.city_count()))
    }
}

pub struct LoadedDatabase {
    text: String,
    city_count: usize,
}

impl LoadedDatabase {
    fn new(text: String, city_count: usize) -> Self {
        Self { text, city_count }
    }

    pub fn iter(&self) -> CityIterator<'_> {
        CityIterator::new(&self.text)
    }

    pub fn find_nearby_cities(
        &self,
        target_city: impl AsRef<str>,
        radius: f32,
    ) -> Option<((&str, Coordinates), Vec<(&str, Coordinates)>)> {
        let target_city = target_city.as_ref();
        let mut backbuf = Vec::with_capacity(self.city_count);
        let mut nearby = Vec::new();
        let mut target = None;
        for (city, coordinates) in self.iter().filter_map(|r| r.ok()) {
            if let Some((_, target_coordinates)) = target {
                let delta: CoordinateDelta = target_coordinates - coordinates;
                if delta.magnitude() <= radius {
                    nearby.push((city, coordinates));
                }
            } else {
                if city.eq_ignore_ascii_case(target_city) {
                    target = Some((city, coordinates));
                } else {
                    backbuf.push((city, coordinates));
                }
            }
        }

        let Some((target_city, target_coordinates)) = target else {
            return None;
        };
        for (city, coordinates) in backbuf {
            let delta: CoordinateDelta = target_coordinates - coordinates;
            if delta.magnitude() <= radius {
                nearby.push((city, coordinates));
            }
        }
        Some(((target_city, target_coordinates), nearby))
    }
}

pub struct CityIterator<'a> {
    lines: Lines<'a>,
}

impl<'a> CityIterator<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            lines: text.lines(),
        }
    }
}

impl<'a> Iterator for CityIterator<'a> {
    type Item = Result<(&'a str, Coordinates), error::LineParseError>;

    fn next(&mut self) -> Option<Self::Item> {
        self.lines.self.lines.next().map(|line| parse_line(line))
    }
}

fn parse_line(line: &str) -> Result<(&str, Coordinates), error::LineParseError> {
    let mut it = line.split('\t');
    let name = it.nth(1).ok_or(error::LineParseError::LineEndedEarly)?;
    let latitude: f32 = it
        .nth(2)
        .ok_or(error::LineParseError::LineEndedEarly)?
        .parse()
        .map_err(error::LineParseError::FloatParseError)?;
    let longitude: f32 = it
        .next()
        .ok_or(error::LineParseError::LineEndedEarly)?
        .parse()
        .map_err(error::LineParseError::FloatParseError)?;

    Ok((name, Coordinates::new(latitude, longitude)))
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Coordinates {
    pub latitude: f32,
    pub longitude: f32,
}

impl Coordinates {
    pub fn new(latitude: f32, longitude: f32) -> Self {
        Self {
            latitude,
            longitude,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CoordinateDelta {
    pub latitude: f32,
    pub longitude: f32,
}

impl CoordinateDelta {
    fn new(latitude: f32, longitude: f32) -> Self {
        Self {
            latitude,
            longitude,
        }
    }

    pub fn magnitude(&self) -> f32 {
        (self.latitude.powf(2.0) + self.longitude.powf(2.0)).sqrt()
    }
}

impl std::ops::Sub for Coordinates {
    type Output = CoordinateDelta;

    fn sub(self, rhs: Self) -> Self::Output {
        CoordinateDelta::new(self.latitude - rhs.latitude, self.longitude - rhs.longitude)
    }
}

pub mod error {

    use std::num::ParseFloatError;

    use zip::result::ZipError;

    use super::*;

    pub enum DatabaseRetrieveError {
        Zip(ZipError),
        Io(std::io::Error),
    }

    pub enum LineParseError {
        FloatParseError(ParseFloatError),
        LineEndedEarly,
    }
}

#[cfg(feature = "static")]
mod embedded {
    pub const CITIES500: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/cities500.zip"));
    pub const CITIES1000: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/cities1000.zip"));
    pub const CITIES5000: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/cities5000.zip"));
    pub const CITIES15000: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/cities15000.zip"));
}
