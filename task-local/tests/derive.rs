use task_local::{TaskLocal, TaskLocalExt};

#[tokio::test]
async fn derive_macro_works() {
    #[derive(TaskLocal)]
    struct Context;

    assert!(Context::try_local(|_| {}).is_err());
    async {
        assert!(Context::local(|context| matches!(context, Context)));
    }
    .with_local(Context)
    .await;
    assert!(Context::try_local(|_| {}).is_err());
}
