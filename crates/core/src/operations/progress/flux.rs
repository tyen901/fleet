use crate::operations::{
    OperationProgressEvent, OperationPublisher, OperationStage, ProgressMetric, ProgressScope,
    ProgressUnit,
};

pub(crate) struct FluxProgressObserver {
    publisher: OperationPublisher,
}

impl FluxProgressObserver {
    pub(crate) fn new(publisher: OperationPublisher) -> Self {
        Self { publisher }
    }
}

impl flux::ProgressObserver for FluxProgressObserver {
    fn update(&self, snapshot: flux::ProgressSnapshot) {
        let (stage, scope, text, unit) = match snapshot.phase {
            flux::MaterializationPhase::Verification => (
                OperationStage::VerifyingInventory,
                ProgressScope::InventoryVerify,
                "Verifying files",
                ProgressUnit::Files,
            ),
            flux::MaterializationPhase::Planning => (
                OperationStage::Sync,
                ProgressScope::MaterializationBytes,
                "Planning materialization",
                ProgressUnit::Paths,
            ),
            flux::MaterializationPhase::StoreDownload
            | flux::MaterializationPhase::ExternalReuse
            | flux::MaterializationPhase::LocalReuse
            | flux::MaterializationPhase::StageWrites => (
                OperationStage::Sync,
                ProgressScope::MaterializationBytes,
                "Materializing files",
                ProgressUnit::Bytes,
            ),
            flux::MaterializationPhase::FinalizeFiles => (
                OperationStage::Sync,
                ProgressScope::MaterializationFiles,
                "Finalizing files",
                ProgressUnit::Files,
            ),
            flux::MaterializationPhase::DeletePaths => (
                OperationStage::CleaningUp,
                ProgressScope::InventoryVerify,
                "Removing managed files",
                ProgressUnit::Files,
            ),
            flux::MaterializationPhase::Inventory => (
                OperationStage::Finalizing,
                ProgressScope::InventoryVerify,
                "Committing inventory",
                ProgressUnit::Paths,
            ),
            flux::MaterializationPhase::Complete | flux::MaterializationPhase::Failed => (
                OperationStage::Finalizing,
                ProgressScope::MaterializationBytes,
                "Finalizing",
                ProgressUnit::Paths,
            ),
        };
        self.publisher.stage(stage);
        self.publisher.progress(OperationProgressEvent {
            stage,
            scope,
            status_text: Some(text.to_string()),
            primary: ProgressMetric {
                label: None,
                done: Some(snapshot.completed),
                total: snapshot.total,
                unit,
            },
            secondary: None,
            throughput_bytes_per_sec: None,
            eta_seconds: None,
        });
    }
}
