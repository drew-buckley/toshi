use ordered_float::OrderedFloat;
use smol_str::SmolStr;
use std::borrow::Cow;
use std::{
    collections::BTreeMap,
    io::{Cursor, Read},
};
use zip::ZipArchive;

#[derive(Debug, Clone, Hash, PartialOrd, Ord, PartialEq, Eq)]
pub struct City {
    name: SmolStr,
    country_code: [u8; 2],
}

impl City {
    pub fn try_new(name: impl Into<SmolStr>, country_code: impl AsRef<str>) -> Option<Self> {
        let country_code = country_code.as_ref().as_bytes();
        if let Ok(country_code) = country_code.try_into() {
            Some(Self::new(name, country_code))
        } else {
            None
        }
    }

    fn new(name: impl Into<SmolStr>, country_code: [u8; 2]) -> Self {
        debug_assert!(std::str::from_utf8(&country_code).is_ok());
        Self {
            name: name.into(),
            country_code,
        }
    }

    pub fn name(&self) -> impl AsRef<str> {
        &self.name
    }

    pub fn country_code(&self) -> &str {
        // safe to unwrap; prevalidated
        std::str::from_utf8(&self.country_code).unwrap()
    }
}

pub struct Database {
    inner: BTreeMap<City, Coordinates>,
}

impl Database {
    fn new(inner: BTreeMap<City, Coordinates>) -> Self {
        Self { inner }
    }

    pub fn iter(&self) -> impl Iterator<Item = (&City, Coordinates)> {
        self.inner.iter().map(|(s, c)| (s, *c))
    }

    pub fn get(&self, city: &City) -> Option<Coordinates> {
        self.inner.get(city).copied()
    }

    pub fn into_btree_map(self) -> BTreeMap<City, Coordinates> {
        self.inner
    }

    pub fn calculate_proximities(&self, city: &City) -> Option<BTreeMap<OrderedFloat<f32>, &City>> {
        self.inner.get(&city).copied().map(|target| {
            self.inner
                .iter()
                .map(|(n, c)| ((target - *c).magnitude().into(), n))
                .collect()
        })
    }
}

impl IntoIterator for Database {
    type IntoIter = std::collections::btree_map::IntoIter<City, Coordinates>;
    type Item = (City, Coordinates);

    fn into_iter(self) -> Self::IntoIter {
        self.inner.into_iter()
    }
}

impl From<Database> for BTreeMap<City, Coordinates> {
    fn from(db: Database) -> Self {
        db.into_btree_map()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DatabaseId {
    Cities500,
    Cities1000,
    Cities5000,
    Cities15000,
}

impl DatabaseId {
    const fn name(self) -> &'static str {
        match self {
            DatabaseId::Cities500 => "cities500",
            DatabaseId::Cities1000 => "cities1000",
            DatabaseId::Cities5000 => "cities5000",
            DatabaseId::Cities15000 => "cities15000",
        }
    }

    pub const fn city_count(self) -> usize {
        match self {
            DatabaseId::Cities500 => 500,
            DatabaseId::Cities1000 => 1000,
            DatabaseId::Cities5000 => 5000,
            DatabaseId::Cities15000 => 15000,
        }
    }
}

cfg_if::cfg_if! {
    if #[cfg(feature = "static")] {
        pub fn cities(
            database: DatabaseId,
        ) -> Result<&'static Database, &'static error::DatabaseError> {
            match database {
                DatabaseId::Cities500 => &static_cache::CITIES500,
                DatabaseId::Cities1000 => &static_cache::CITIES1000,
                DatabaseId::Cities5000 => &static_cache::CITIES5000,
                DatabaseId::Cities15000 => &static_cache::CITIES15000,
            }
            .as_ref()
        }

        fn zip_bytes(database: DatabaseId) -> Result<Cow<'static, [u8]>, error::RetrieveError> {
            Ok(match database {
                DatabaseId::Cities500 => embedded::CITIES500,
                DatabaseId::Cities1000 => embedded::CITIES1000,
                DatabaseId::Cities5000 => embedded::CITIES5000,
                DatabaseId::Cities15000 => embedded::CITIES15000,
            }
            .into())
        }
    } else if #[cfg(feature = "fetch")] {
        pub fn fetch_cities(
            database: DatabaseId,
        ) -> Result<std::sync::Arc<Database>, error::DatabaseError> {
            let mut cache = match database {
                DatabaseId::Cities500 => &fetch_cache::CITIES500,
                DatabaseId::Cities1000 => &fetch_cache::CITIES1000,
                DatabaseId::Cities5000 => &fetch_cache::CITIES5000,
                DatabaseId::Cities15000 => &fetch_cache::CITIES15000,
            }.lock();

            if let Some(cache) = cache.clone() {
                Ok(cache)
            } else {
                let db = std::sync::Arc::new(retrieve(database)?);
                *cache = Some(db.clone());
                Ok(db)
            }
        }

        fn zip_bytes(database: DatabaseId) -> Result<Cow<'static, [u8]>, error::RetrieveError> {
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

fn retrieve(database: DatabaseId) -> Result<Database, error::DatabaseError> {
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
        Ok(Database::new(cities))
    } else {
        Err(error::DatabaseError::PartialLineParse(cities, parse_errors))
    }
}

fn parse_line(line: &str) -> Result<(City, Coordinates), error::LineParseError> {
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

    let country_code: [u8; 2] = it
        .nth(2)
        .ok_or(error::LineParseError::LineEndedEarly)?
        .as_bytes()
        .try_into()
        .map_err(|_| error::LineParseError::CountryCodeBadSize)?;

    Ok((
        City::new(name, country_code),
        Coordinates::new(latitude, longitude),
    ))
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
        PartialLineParse(BTreeMap<City, Coordinates>, Vec<LineParseError>),
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
        CountryCodeBadSize,
        LineEndedEarly,
    }

    impl std::fmt::Display for LineParseError {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            match self {
                Self::FloatParseError(..) => write!(f, "failed to parse float"),
                Self::CountryCodeBadSize => write!(f, "country code must be 2 bytes"),
                Self::LineEndedEarly => write!(f, "line ended early"),
            }
        }
    }

    impl std::error::Error for LineParseError {
        fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
            match self {
                Self::FloatParseError(err) => Some(err),
                Self::CountryCodeBadSize | Self::LineEndedEarly => None,
            }
        }
    }
}

#[cfg(feature = "static")]
mod embedded {
    pub const CITIES500: &[u8] = include_bytes!("assets/cities500.zip");
    pub const CITIES1000: &[u8] = include_bytes!("assets/cities1000.zip");
    pub const CITIES5000: &[u8] = include_bytes!("assets/cities5000.zip");
    pub const CITIES15000: &[u8] = include_bytes!("assets/cities15000.zip");
}

#[cfg(feature = "static")]
mod static_cache {
    use std::sync::LazyLock;

    use super::*;

    pub type Cache = LazyLock<Result<Database, error::DatabaseError>>;

    pub static CITIES500: Cache = LazyLock::new(|| retrieve(DatabaseId::Cities500));
    pub static CITIES1000: Cache = LazyLock::new(|| retrieve(DatabaseId::Cities1000));
    pub static CITIES5000: Cache = LazyLock::new(|| retrieve(DatabaseId::Cities5000));
    pub static CITIES15000: Cache = LazyLock::new(|| retrieve(DatabaseId::Cities15000));
}

#[cfg(feature = "fetch")]
mod fetch_cache {
    use std::sync::{Arc, LazyLock};

    use parking_lot::Mutex;

    use super::*;

    pub type Cache = LazyLock<Mutex<Option<Arc<Database>>>>;

    pub static CITIES500: Cache = LazyLock::new(|| Mutex::new(None));
    pub static CITIES1000: Cache = LazyLock::new(|| Mutex::new(None));
    pub static CITIES5000: Cache = LazyLock::new(|| Mutex::new(None));
    pub static CITIES15000: Cache = LazyLock::new(|| Mutex::new(None));
}
