use task_global::{TaskGlobal, TaskGlobalExt};

#[tokio::test]
async fn derive_macro_works() {
    #[derive(TaskGlobal)]
    struct Context;

    assert!(Context::try_global(|_| {}).is_err());
    async {
        assert!(Context::global(|context| matches!(context, Context)));
    }
    .with_global(Context)
    .await;
    assert!(Context::try_global(|_| {}).is_err());
}
