// Simple integration test for model
use lievo::model::{AnalysisStatus, EntityTier, RelType};
use std::str::FromStr;

#[test]
fn test_model_integration() {
    // Test EntityTier
    assert_eq!(EntityTier::Module.to_string(), "module");
    assert_eq!(EntityTier::from_str("module").unwrap(), EntityTier::Module);

    // Test RelType
    assert_eq!(RelType::DependsOn.to_string(), "depends_on");
    assert_eq!(RelType::from_str("depends_on").unwrap(), RelType::DependsOn);

    // Test AnalysisStatus
    assert_eq!(AnalysisStatus::Completed.to_string(), "completed");
    assert_eq!(
        AnalysisStatus::from_str("completed").unwrap(),
        AnalysisStatus::Completed
    );
}
