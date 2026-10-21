#[cfg(feature = "static")]
use std::borrow::Cow;
use std::{
    borrow::Cow,
    collections::BTreeMap,
    io::{Cursor, Read, Seek},
    str::Lines,
};
use zip::ZipArchive;

cfg_if::cfg_if! {
    if #[cfg(feature = "arc")] {
        pub type Trc<T> = std::sync::Arc<T>;
    } else {
        pub type Trc<T> = std::rc::Rc<T>;
    }
}

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

cfg_if::cfg_if! {
    if #[cfg(feature = "static")] {
        fn zip_bytes(database: Database) -> Result<Cow<'static, [u8]>, error::RetrieveError> {
            Ok(match database {
                Database::Cities500 => embedded::CITIES500,
                Database::Cities1000 => embedded::CITIES1000,
                Database::Cities5000 => embedded::CITIES5000,
                Database::Cities15000 => embedded::CITIES15000,
            }
            .into())
        }
    } else if #[cfg(feature = "fetch")] {
        fn zip_bytes(database: Database) -> Result<Cow<'static, [u8]>, error::RetrieveError> {
            let body = ureq::get(format!(
                "https://download.geonames.org/export/dump/{}.zip",
                database.name()
            ))
            .call().map_err(error::RetrieveError::Fetch)?
            .into_body();

            let mut buf = Vec::with_capacity(body.content_length().unwrap_or(0) as usize);
            body.into_reader().read_to_end(&mut buf).map_err(error::RetrieveError::Io)?;

            Ok(buf.into())
        }
    }
}

fn retrieve(database: Database) -> Result<BTreeMap<String, Coordinates>, error::DatabaseError> {
    let mut zip = ZipArchive::new(Cursor::new(
        zip_bytes(database).map_err(error::DatabaseError::Retrieval)?,
    ))
    .map_err(error::RetrieveError::Zip)
    .map_err(error::DatabaseError::Retrieval)?;
    let mut file = zip
        .by_name(&format!("{}.txt", database.name()))
        .map_err(error::RetrieveError::Zip)
        .map_err(error::DatabaseError::Retrieval)?;

    let mut raw = String::with_capacity(file.size() as usize);
    file.read_to_string(&mut raw)
        .map_err(error::RetrieveError::Io)
        .map_err(error::DatabaseError::Retrieval)?;

    let mut parse_errors = Vec::new();
    let mut cities = BTreeMap::new();
    for res in raw.lines().map(|l| parse_line(l)) {
        match res {
            Ok((name, coordinates)) => {
                cities.insert(name.to_owned(), coordinates);
            }
            Err(err) => {
                parse_errors.push(err);
            }
        }
    }

    if parse_errors.is_empty() {
        Ok(cities)
    } else {
        Err(error::DatabaseError::PartialLineParse(cities, parse_errors))
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
        self.lines.next().map(|line| parse_line(line))
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

    use std::{collections::BTreeMap, num::ParseFloatError};

    use zip::result::ZipError;

    use super::*;

    #[derive(Debug)]
    pub enum DatabaseError {
        Retrieval(RetrieveError),
        PartialLineParse(BTreeMap<String, Coordinates>, Vec<LineParseError>),
    }

    impl std::fmt::Display for DatabaseError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                Self::Retrieval(..) => write!(f, "failed to retrieve database bytes"),
                Self::PartialLineParse(..) => write!(f, "failed parse some or all database lines"),
            }
        }
    }

    impl std::error::Error for DatabaseError {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            match self {
                Self::Retrieval(err) => Some(err),
                Self::PartialLineParse(_, err) => err.first().map(|e| e as &dyn std::error::Error),
            }
        }
    }

    #[derive(Debug)]
    pub enum RetrieveError {
        #[cfg(feature = "fetch")]
        Fetch(ureq::Error),
        Zip(ZipError),
        Io(std::io::Error),
    }

    impl std::fmt::Display for RetrieveError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                #[cfg(feature = "fetch")]
                Self::Fetch(..) => write!(f, "failed to fetch database"),
                Self::Zip(..) => write!(f, "failed to extract database"),
                Self::Io(..) => write!(f, "IO error during database retrieval"),
            }
        }
    }

    impl std::error::Error for RetrieveError {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            match self {
                #[cfg(feature = "fetch")]
                Self::Fetch(err) => Some(err),
                Self::Zip(err) => Some(err),
                Self::Io(err) => Some(err),
            }
        }
    }

    #[derive(Debug)]
    pub enum LineParseError {
        FloatParseError(ParseFloatError),
        LineEndedEarly,
    }

    impl std::fmt::Display for LineParseError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                Self::FloatParseError(..) => write!(f, "failed to parse float"),
                Self::LineEndedEarly => write!(f, "line ended early"),
            }
        }
    }

    impl std::error::Error for LineParseError {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            match self {
                Self::FloatParseError(err) => Some(err),
                Self::LineEndedEarly => None,
            }
        }
    }
}

#[cfg(feature = "static")]
mod embedded {
    pub const CITIES500: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/cities500.zip"));
    pub const CITIES1000: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/cities1000.zip"));
    pub const CITIES5000: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/cities5000.zip"));
    pub const CITIES15000: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/cities15000.zip"));
}

mod cache {
    use std::sync::LazyLock;

    use super::*;

    pub type Cache = LazyLock<Result<BTreeMap<String, Coordinates>, error::DatabaseError>>;

    pub static CITIES500: Cache = LazyLock::new(|| retrieve(Database::Cities500));
    pub static CITIES1000: Cache = LazyLock::new(|| retrieve(Database::Cities1000));
    pub static CITIES5000: Cache = LazyLock::new(|| retrieve(Database::Cities5000));
    pub static CITIES15000: Cache = LazyLock::new(|| retrieve(Database::Cities15000));
}
