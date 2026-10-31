use crate::error::Result;
use crate::model::Feature;
#[cfg(feature = "cli")]
use crate::output::OutputWriter;

pub(super) trait FeatureSink {
    fn write_feature(&mut self, feature: Feature) -> Result<()>;
}

#[cfg(feature = "cli")]
impl FeatureSink for OutputWriter {
    fn write_feature(&mut self, feature: Feature) -> Result<()> {
        OutputWriter::write_feature(self, &feature)
    }
}

#[derive(Debug, Clone)]
pub(super) struct VecFeatureSink {
    pub(super) features: Vec<Feature>,
}

impl VecFeatureSink {
    pub(super) fn new() -> Self {
        Self {
            features: Vec::new(),
        }
    }
}

impl FeatureSink for VecFeatureSink {
    fn write_feature(&mut self, feature: Feature) -> Result<()> {
        self.features.push(feature);
        Ok(())
    }
}
