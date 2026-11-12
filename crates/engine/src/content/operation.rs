use crate::content::tokenizer::ContentToken;

#[derive(Debug, Clone, PartialEq)]
pub enum Operand {
    Null,
    Integer(i64),
    Real(f64),
    Boolean(bool),
    Name(String),
    String(Vec<u8>),
    Array(Vec<Operand>),
    /// A content-stream dictionary. Inline-image `/DecodeParms` commonly uses
    /// this form; keeping it distinct from arrays is security-critical because
    /// predictor parameters must be paired with the intended filter.
    Dictionary(Vec<(String, Operand)>),
}

impl Operand {
    pub fn as_integer(&self) -> Option<i64> {
        match self {
            Self::Integer(value) => Some(*value),
            _ => None,
        }
    }

    pub fn as_real(&self) -> Option<f64> {
        match self {
            Self::Real(value) => Some(*value),
            _ => None,
        }
    }

    pub fn as_number(&self) -> Option<f64> {
        match self {
            Self::Integer(value) => Some(*value as f64),
            Self::Real(value) => Some(*value),
            _ => None,
        }
    }

    pub fn as_name(&self) -> Option<&str> {
        match self {
            Self::Name(value) => Some(value),
            _ => None,
        }
    }

    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Self::String(value) => Some(value),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[Operand]> {
        match self {
            Self::Array(value) => Some(value),
            _ => None,
        }
    }

    pub fn as_dictionary(&self) -> Option<&[(String, Operand)]> {
        match self {
            Self::Dictionary(value) => Some(value),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Boolean(value) => Some(*value),
            _ => None,
        }
    }
}

impl From<ContentToken> for Option<Operand> {
    fn from(value: ContentToken) -> Self {
        match value {
            ContentToken::Null => Some(Operand::Null),
            ContentToken::Integer(value) => Some(Operand::Integer(value)),
            ContentToken::Real(value) => Some(Operand::Real(value)),
            ContentToken::Boolean(value) => Some(Operand::Boolean(value)),
            ContentToken::Name(value) => Some(Operand::Name(value)),
            ContentToken::LiteralString(value) | ContentToken::HexString(value) => {
                Some(Operand::String(value))
            }
            ContentToken::InlineImageData(value) => Some(Operand::String(value)),
            ContentToken::ArrayStart
            | ContentToken::ArrayEnd
            | ContentToken::DictStart
            | ContentToken::DictEnd
            | ContentToken::Operator(_) => None,
        }
    }
}

/// A content-stream operator name.
///
/// Page programs hold one operation per operator in the stream (hundreds of
/// thousands on dense map pages), so the name is stored without a heap
/// allocation: every operator the PDF specification defines (plus the
/// parser's synthetic `inline_image_data`) is interned as a `&'static str`,
/// and only unknown names are boxed. It dereferences to `str` and compares
/// with `str`/`String`, so it reads like the `String` it replaces.
#[derive(Clone)]
pub struct Operator(OperatorRepr);

#[derive(Clone)]
enum OperatorRepr {
    Known(&'static str),
    // Boxed twice so the type stays two words: the thin outer pointer fits
    // beside the `Known` slice pointer's niche.
    #[allow(clippy::box_collection, clippy::redundant_allocation)]
    Other(Box<Box<str>>),
}

/// The interned spelling of a PDF content operator.
fn known_operator(name: &str) -> Option<&'static str> {
    Some(match name {
        "b" => "b",
        "B" => "B",
        "b*" => "b*",
        "B*" => "B*",
        "BDC" => "BDC",
        "BI" => "BI",
        "BMC" => "BMC",
        "BT" => "BT",
        "BX" => "BX",
        "c" => "c",
        "cm" => "cm",
        "CS" => "CS",
        "cs" => "cs",
        "d" => "d",
        "d0" => "d0",
        "d1" => "d1",
        "Do" => "Do",
        "DP" => "DP",
        "EI" => "EI",
        "EMC" => "EMC",
        "ET" => "ET",
        "EX" => "EX",
        "f" => "f",
        "F" => "F",
        "f*" => "f*",
        "G" => "G",
        "g" => "g",
        "gs" => "gs",
        "h" => "h",
        "i" => "i",
        "ID" => "ID",
        "j" => "j",
        "J" => "J",
        "K" => "K",
        "k" => "k",
        "l" => "l",
        "m" => "m",
        "M" => "M",
        "MP" => "MP",
        "n" => "n",
        "q" => "q",
        "Q" => "Q",
        "re" => "re",
        "RG" => "RG",
        "rg" => "rg",
        "ri" => "ri",
        "s" => "s",
        "S" => "S",
        "SC" => "SC",
        "sc" => "sc",
        "SCN" => "SCN",
        "scn" => "scn",
        "sh" => "sh",
        "T*" => "T*",
        "Tc" => "Tc",
        "Td" => "Td",
        "TD" => "TD",
        "Tf" => "Tf",
        "Tj" => "Tj",
        "TJ" => "TJ",
        "TL" => "TL",
        "Tm" => "Tm",
        "Tr" => "Tr",
        "Ts" => "Ts",
        "Tw" => "Tw",
        "Tz" => "Tz",
        "v" => "v",
        "w" => "w",
        "W" => "W",
        "W*" => "W*",
        "y" => "y",
        "'" => "'",
        "\"" => "\"",
        "inline_image_data" => "inline_image_data",
        _ => return None,
    })
}

impl Operator {
    pub fn new(name: &str) -> Self {
        match known_operator(name) {
            Some(known) => Self(OperatorRepr::Known(known)),
            None => Self(OperatorRepr::Other(Box::new(name.into()))),
        }
    }

    /// The operator spelled by `bytes` (invalid UTF-8 is replaced, as the
    /// tokenizer always did); interned operators need no allocation.
    pub fn from_bytes(bytes: &[u8]) -> Self {
        match std::str::from_utf8(bytes) {
            Ok(name) => Self::new(name),
            Err(_) => Self::from(String::from_utf8_lossy(bytes).into_owned()),
        }
    }

    pub fn as_str(&self) -> &str {
        match &self.0 {
            OperatorRepr::Known(name) => name,
            OperatorRepr::Other(name) => name,
        }
    }

    /// Heap bytes the name holds (zero for interned operators).
    pub fn heap_bytes(&self) -> usize {
        match &self.0 {
            OperatorRepr::Known(_) => 0,
            OperatorRepr::Other(name) => name.len(),
        }
    }
}

impl std::ops::Deref for Operator {
    type Target = str;

    fn deref(&self) -> &str {
        self.as_str()
    }
}

impl AsRef<str> for Operator {
    fn as_ref(&self) -> &str {
        self.as_str()
    }
}

impl std::borrow::Borrow<str> for Operator {
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

impl From<&str> for Operator {
    fn from(name: &str) -> Self {
        Self::new(name)
    }
}

impl From<String> for Operator {
    fn from(name: String) -> Self {
        match known_operator(&name) {
            Some(known) => Self(OperatorRepr::Known(known)),
            None => Self(OperatorRepr::Other(Box::new(name.into_boxed_str()))),
        }
    }
}

impl From<&String> for Operator {
    fn from(name: &String) -> Self {
        Self::new(name)
    }
}

impl From<&Operator> for Operator {
    fn from(operator: &Operator) -> Self {
        operator.clone()
    }
}

impl From<Operator> for String {
    fn from(operator: Operator) -> Self {
        operator.as_str().to_string()
    }
}

impl std::fmt::Debug for Operator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        std::fmt::Debug::fmt(self.as_str(), f)
    }
}

impl std::fmt::Display for Operator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl PartialEq for Operator {
    fn eq(&self, other: &Self) -> bool {
        self.as_str() == other.as_str()
    }
}

impl Eq for Operator {}

impl std::hash::Hash for Operator {
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        self.as_str().hash(state)
    }
}

impl PartialOrd for Operator {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for Operator {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.as_str().cmp(other.as_str())
    }
}

impl PartialEq<str> for Operator {
    fn eq(&self, other: &str) -> bool {
        self.as_str() == other
    }
}

impl PartialEq<&str> for Operator {
    fn eq(&self, other: &&str) -> bool {
        self.as_str() == *other
    }
}

impl PartialEq<String> for Operator {
    fn eq(&self, other: &String) -> bool {
        self.as_str() == other.as_str()
    }
}

impl PartialEq<Operator> for str {
    fn eq(&self, other: &Operator) -> bool {
        self == other.as_str()
    }
}

impl PartialEq<Operator> for &str {
    fn eq(&self, other: &Operator) -> bool {
        *self == other.as_str()
    }
}

impl PartialEq<Operator> for String {
    fn eq(&self, other: &Operator) -> bool {
        self.as_str() == other.as_str()
    }
}

/// The operands of one content operation.
///
/// Up to two operands — the `m`/`l` path construction that dominates dense
/// vector pages, `Td`, `Tf`, and every one-operand operator — are stored
/// inline, so most operations need no operand allocation at all; longer
/// lists keep a `Vec`. The type is 64 bytes (the inline pair; the variant
/// tag lives in the first operand's niche). It dereferences to
/// `[Operand]`, so indexing, `len`, `iter`, `first`, `get` and slicing work
/// as on the `Vec` it replaces.
#[derive(Clone, Default)]
pub struct Operands(OperandsRepr);

#[derive(Clone, Default)]
enum OperandsRepr {
    #[default]
    Empty,
    One(Operand),
    Two([Operand; 2]),
    Many(Vec<Operand>),
}

impl Operands {
    pub fn new() -> Self {
        Self::default()
    }

    /// Room for `capacity` operands: inline up to two, otherwise one
    /// allocation of exactly that size.
    pub fn with_capacity(capacity: usize) -> Self {
        if capacity > 2 {
            Self(OperandsRepr::Many(Vec::with_capacity(capacity)))
        } else {
            Self::new()
        }
    }

    pub fn as_slice(&self) -> &[Operand] {
        match &self.0 {
            OperandsRepr::Empty => &[],
            OperandsRepr::One(operand) => std::slice::from_ref(operand),
            OperandsRepr::Two(pair) => pair,
            OperandsRepr::Many(operands) => operands,
        }
    }

    pub fn as_mut_slice(&mut self) -> &mut [Operand] {
        match &mut self.0 {
            OperandsRepr::Empty => &mut [],
            OperandsRepr::One(operand) => std::slice::from_mut(operand),
            OperandsRepr::Two(pair) => pair,
            OperandsRepr::Many(operands) => operands,
        }
    }

    pub fn into_vec(self) -> Vec<Operand> {
        match self.0 {
            OperandsRepr::Empty => Vec::new(),
            OperandsRepr::One(operand) => vec![operand],
            OperandsRepr::Two([first, second]) => vec![first, second],
            OperandsRepr::Many(operands) => operands,
        }
    }

    pub fn push(&mut self, operand: Operand) {
        let repr = std::mem::take(&mut self.0);
        self.0 = match repr {
            OperandsRepr::Empty => OperandsRepr::One(operand),
            OperandsRepr::One(first) => OperandsRepr::Two([first, operand]),
            OperandsRepr::Two([first, second]) => OperandsRepr::Many(vec![first, second, operand]),
            OperandsRepr::Many(mut operands) => {
                operands.push(operand);
                OperandsRepr::Many(operands)
            }
        };
    }

    /// Remove the operands in `range` and return them, keeping the rest.
    pub fn drain(
        &mut self,
        range: impl std::ops::RangeBounds<usize>,
    ) -> std::vec::IntoIter<Operand> {
        let mut operands = std::mem::take(self).into_vec();
        let drained: Vec<Operand> = operands.drain(range).collect();
        *self = Self::from(operands);
        drained.into_iter()
    }

    pub fn clear(&mut self) {
        self.0 = OperandsRepr::Empty;
    }

    /// Heap bytes of the operand list itself (zero while stored inline);
    /// what the operands own (names, strings, arrays) is not included.
    pub fn heap_bytes(&self) -> usize {
        match &self.0 {
            OperandsRepr::Many(operands) => operands
                .capacity()
                .saturating_mul(std::mem::size_of::<Operand>()),
            _ => 0,
        }
    }
}

impl std::ops::Deref for Operands {
    type Target = [Operand];

    fn deref(&self) -> &[Operand] {
        self.as_slice()
    }
}

impl std::ops::DerefMut for Operands {
    fn deref_mut(&mut self) -> &mut [Operand] {
        self.as_mut_slice()
    }
}

impl From<Vec<Operand>> for Operands {
    fn from(operands: Vec<Operand>) -> Self {
        if operands.len() > 2 {
            return Self(OperandsRepr::Many(operands));
        }
        let mut iter = operands.into_iter();
        Self(match (iter.next(), iter.next()) {
            (None, _) => OperandsRepr::Empty,
            (Some(first), None) => OperandsRepr::One(first),
            (Some(first), Some(second)) => OperandsRepr::Two([first, second]),
        })
    }
}

impl From<Operands> for Vec<Operand> {
    fn from(operands: Operands) -> Self {
        operands.into_vec()
    }
}

impl FromIterator<Operand> for Operands {
    fn from_iter<I: IntoIterator<Item = Operand>>(iter: I) -> Self {
        let mut operands = Self::new();
        for operand in iter {
            operands.push(operand);
        }
        operands
    }
}

impl Extend<Operand> for Operands {
    fn extend<I: IntoIterator<Item = Operand>>(&mut self, iter: I) {
        for operand in iter {
            self.push(operand);
        }
    }
}

impl IntoIterator for Operands {
    type Item = Operand;
    type IntoIter = std::vec::IntoIter<Operand>;

    fn into_iter(self) -> Self::IntoIter {
        self.into_vec().into_iter()
    }
}

impl<'a> IntoIterator for &'a Operands {
    type Item = &'a Operand;
    type IntoIter = std::slice::Iter<'a, Operand>;

    fn into_iter(self) -> Self::IntoIter {
        self.as_slice().iter()
    }
}

impl<'a> IntoIterator for &'a mut Operands {
    type Item = &'a mut Operand;
    type IntoIter = std::slice::IterMut<'a, Operand>;

    fn into_iter(self) -> Self::IntoIter {
        self.as_mut_slice().iter_mut()
    }
}

impl std::fmt::Debug for Operands {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.as_slice()).finish()
    }
}

impl PartialEq for Operands {
    fn eq(&self, other: &Self) -> bool {
        self.as_slice() == other.as_slice()
    }
}

impl PartialEq<Vec<Operand>> for Operands {
    fn eq(&self, other: &Vec<Operand>) -> bool {
        self.as_slice() == other.as_slice()
    }
}

impl PartialEq<[Operand]> for Operands {
    fn eq(&self, other: &[Operand]) -> bool {
        self.as_slice() == other
    }
}

#[derive(Debug, Clone)]
pub struct ContentOperation {
    pub operator: Operator,
    pub operands: Operands,
}

impl ContentOperation {
    pub fn new(operator: impl Into<Operator>, operands: Vec<Operand>) -> Self {
        Self {
            operator: operator.into(),
            operands: Operands::from(operands),
        }
    }

    pub fn from_parts(operator: Operator, operands: Operands) -> Self {
        Self { operator, operands }
    }

    pub fn operand(&self, n: usize) -> Option<&Operand> {
        self.operands.get(n)
    }

    pub fn number(&self, n: usize) -> Option<f64> {
        self.operand(n).and_then(Operand::as_number)
    }

    pub fn name(&self, n: usize) -> Option<&str> {
        self.operand(n).and_then(Operand::as_name)
    }

    pub fn string_bytes(&self, n: usize) -> Option<&[u8]> {
        self.operand(n).and_then(Operand::as_bytes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn operators_are_interned_and_compare_like_strings() {
        let line = Operator::from("l".to_string());
        assert!(matches!(line.0, OperatorRepr::Known("l")));
        assert_eq!(line, "l");
        assert_eq!(line.as_str(), "l");
        assert_eq!(String::from(line.clone()), "l");
        assert_eq!(format!("{line} {line:?}"), "l \"l\"");
        let unknown = Operator::from("frobnicate");
        assert!(matches!(unknown.0, OperatorRepr::Other(_)));
        assert_eq!(unknown, "frobnicate".to_string());
        assert!(matches!(&*unknown, "frobnicate"));
        assert_eq!(Operator::from("q"), Operator::from(&"q".to_string()));
    }

    #[test]
    fn operands_store_up_to_two_inline() {
        assert!(std::mem::size_of::<Operands>() <= 2 * std::mem::size_of::<Operand>());
        assert_eq!(std::mem::size_of::<Operator>(), 16);
        let pair = Operands::from(vec![Operand::Integer(1), Operand::Real(2.5)]);
        assert!(matches!(pair.0, OperandsRepr::Two(_)));
        assert_eq!(pair.len(), 2);
        assert_eq!(pair[1], Operand::Real(2.5));
        let mut grown = pair.clone();
        grown.push(Operand::Null);
        assert!(matches!(grown.0, OperandsRepr::Many(_)));
        assert_eq!(
            grown,
            vec![Operand::Integer(1), Operand::Real(2.5), Operand::Null]
        );
        let drained: Vec<Operand> = grown.drain(0..2).collect();
        assert_eq!(drained, vec![Operand::Integer(1), Operand::Real(2.5)]);
        assert!(matches!(grown.0, OperandsRepr::One(Operand::Null)));
        assert_eq!(Operands::new().into_vec(), Vec::<Operand>::new());
        let collected: Operands = (0..4).map(Operand::Integer).collect();
        assert_eq!(collected.into_vec().len(), 4);
    }
}
