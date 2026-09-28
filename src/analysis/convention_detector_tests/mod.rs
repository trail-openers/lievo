pub mod helpers;
pub mod pattern_tests;

use crate::analysis::convention_detector::ConventionDetector;
use crate::model::EntityTier;
use helpers::*;

#[test]
fn test_is_snake_case() {
    use crate::analysis::convention_detector::is_snake_case;
    assert!(is_snake_case("my_function"));
    assert!(is_snake_case("my_variable_name"));
    assert!(!is_snake_case("myFunction"));
    assert!(!is_snake_case("MyFunction"));
    assert!(!is_snake_case("My_Function"));
}

#[test]
fn test_is_camel_case() {
    use crate::analysis::convention_detector::is_camel_case;
    assert!(is_camel_case("myFunction"));
    assert!(is_camel_case("myVariableName"));
    assert!(!is_camel_case("my_function"));
    assert!(!is_camel_case("MyFunction"));
    assert!(!is_camel_case("MY_FUNCTION"));
}

#[test]
fn test_empty_string() {
    use crate::analysis::convention_detector::{is_camel_case, is_snake_case};
    assert!(!is_snake_case(""));
    assert!(!is_camel_case(""));
}

#[test]
fn test_detect_naming_convention_all_snake_case() {
    let entities = vec![
        make_entity("fn1", "proj1", EntityTier::Function, "get_user", None),
        make_entity("fn2", "proj1", EntityTier::Function, "set_config", None),
        make_entity("fn3", "proj1", EntityTier::Function, "parse_json", None),
    ];

    let storage = MockStorage::new(entities);
    let detector = ConventionDetector::new(&storage);
    let result = detector.detect_naming_convention(&storage.entities, "proj1");

    assert!(result.is_some());
    let conv = result.unwrap();
    assert_eq!(conv.category, "naming");
    assert!(conv.title.contains("snake_case"));
    assert_eq!(conv.confidence, 1.0);
    assert_eq!(conv.example_code, Some("get_user".to_string()));
}

#[test]
fn test_detect_naming_convention_all_camel_case() {
    let entities = vec![
        make_entity("fn1", "proj1", EntityTier::Function, "getUser", None),
        make_entity("fn2", "proj1", EntityTier::Function, "setConfig", None),
        make_entity("fn3", "proj1", EntityTier::Function, "parseJson", None),
    ];

    let storage = MockStorage::new(entities);
    let detector = ConventionDetector::new(&storage);
    let result = detector.detect_naming_convention(&storage.entities, "proj1");

    assert!(result.is_some());
    let conv = result.unwrap();
    assert_eq!(conv.category, "naming");
    assert!(conv.title.contains("camelCase"));
    assert_eq!(conv.confidence, 1.0);
    assert_eq!(conv.example_code, Some("getUser".to_string()));
}

#[test]
fn test_detect_naming_convention_mixed_no_pattern() {
    let entities = vec![
        make_entity("fn1", "proj1", EntityTier::Function, "get_user", None),
        make_entity("fn2", "proj1", EntityTier::Function, "setConfig", None),
        make_entity("fn3", "proj1", EntityTier::Function, "parseData", None),
    ];

    let storage = MockStorage::new(entities);
    let detector = ConventionDetector::new(&storage);
    let result = detector.detect_naming_convention(&storage.entities, "proj1");

    assert!(result.is_none());
}

#[test]
fn test_detect_naming_convention_empty_entities() {
    let entities = vec![];

    let storage = MockStorage::new(entities);
    let detector = ConventionDetector::new(&storage);
    let result = detector.detect_naming_convention(&storage.entities, "proj1");

    assert!(result.is_none());
}

#[test]
fn test_detect_naming_convention_only_modules() {
    let entities = vec![
        make_entity("mod1", "proj1", EntityTier::Module, "auth_service", None),
        make_entity("mod2", "proj1", EntityTier::Module, "user_management", None),
        make_entity("file1", "proj1", EntityTier::File, "data.rs", None),
    ];

    let storage = MockStorage::new(entities);
    let detector = ConventionDetector::new(&storage);
    let result = detector.detect_naming_convention(&storage.entities, "proj1");

    assert!(result.is_some());
    let conv = result.unwrap();
    assert!(conv.title.contains("snake_case"));
}
