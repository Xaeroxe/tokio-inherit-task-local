use tokio_inherit_task_local::{inheritable_task_local, FutureInheritTaskLocal};

inheritable_task_local! {
    pub static TEST_VALUE: u32;
    pub static ANOTHER_TEST_VALUE: u32;
}

#[tokio::test]
async fn basic() {
    let out = TEST_VALUE.scope(5, async { TEST_VALUE.with(|&v| v) }).await;
    assert_eq!(out, 5);
}

#[tokio::test]
async fn basic_inherit() {
    let out = TEST_VALUE
        .scope(5, async {
            tokio::spawn(async { TEST_VALUE.with(|&v| v) }.inherit_task_local()).await
        })
        .await
        .unwrap();
    assert_eq!(out, 5);
}

#[tokio::test]
async fn inherit_repeatedly() {
    let out = TEST_VALUE
        .scope(5, async {
            tokio::spawn(async {
                tokio::spawn(async { TEST_VALUE.with(|&v| v) }.inherit_task_local())
            }.inherit_task_local()).await
        })
        .await
        .unwrap()
        .await
        .unwrap();
    assert_eq!(out, 5);
}

#[tokio::test]
#[should_panic]
async fn not_inherited_if_future_not_wrapped() {
    let out = TEST_VALUE
        .scope(5, async {
            tokio::spawn(async { TEST_VALUE.with(|&v| v) }).await
        })
        .await
        .unwrap();
    assert_eq!(out, 5);
}


#[tokio::test]
#[should_panic]
async fn not_inherited_repeatedly_if_chain_broken() {
    let out = TEST_VALUE
        .scope(5, async {
            tokio::spawn(async {
                tokio::spawn(async { TEST_VALUE.with(|&v| v) }.inherit_task_local())
            }).await
        })
        .await
        .unwrap()
        .await
        .unwrap();
    assert_eq!(out, 5);
}

#[tokio::test]
async fn use_another_test_value() {
    let out = ANOTHER_TEST_VALUE.scope(5, async { ANOTHER_TEST_VALUE.with(|&v| v) }).await;
    assert_eq!(out, 5);
}
