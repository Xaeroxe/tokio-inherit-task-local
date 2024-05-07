use tokio_inherit_task_local::{
    inheritable_task_local, FutureInheritTaskLocal, InheritableAccessError,
};

inheritable_task_local! {
    pub static TEST_VALUE: u32;
    pub static ANOTHER_TEST_VALUE: String;
}

#[tokio::test]
async fn basic() {
    let out = TEST_VALUE.scope(5, async { TEST_VALUE.with(|&v| v) }).await;
    assert_eq!(out, 5);
}

#[tokio::test]
async fn basic_try_with() {
    let out = TEST_VALUE
        .scope(5, async { TEST_VALUE.try_with(|&v| v) })
        .await
        .unwrap();
    assert_eq!(out, 5);
}

#[tokio::test]
async fn fail_try_with() {
    let out = async { TEST_VALUE.try_with(|&v| v) }.await.unwrap_err();
    assert_eq!(out, InheritableAccessError::NotInTokio);
}

#[tokio::test]
async fn fail_try_with_use_both() {
    let out = ANOTHER_TEST_VALUE
        .scope(String::from("foo"), async { TEST_VALUE.try_with(|&v| v) })
        .await
        .unwrap_err();
    assert_eq!(out, InheritableAccessError::NotInTable);
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
            tokio::spawn(
                async {
                    tokio::spawn(async { TEST_VALUE.with(|&v| v) }.inherit_task_local()).await
                }
                .inherit_task_local(),
            )
            .await
        })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(out, 5);
}

#[tokio::test]
async fn basic_sync() {
    let out = TEST_VALUE.sync_scope(5, || TEST_VALUE.with(|&v| v));
    assert_eq!(out, 5);
}

#[tokio::test]
async fn basic_sync_use_both() {
    let (uint, str) = TEST_VALUE.sync_scope(5, || {
        ANOTHER_TEST_VALUE.sync_scope(String::from("foo"), || {
            TEST_VALUE.with(|&v| ANOTHER_TEST_VALUE.with(|str| (v, str.clone())))
        })
    });
    assert_eq!(uint, 5);
    assert_eq!(str, "foo");
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
                tokio::spawn(async { TEST_VALUE.with(|&v| v) }.inherit_task_local()).await
            })
            .await
        })
        .await
        .unwrap()
        .unwrap();
    assert_eq!(out, 5);
}

#[tokio::test]
async fn use_another_test_value() {
    let out = ANOTHER_TEST_VALUE
        .scope(String::from("foo"), async {
            ANOTHER_TEST_VALUE.with(|v| v.clone())
        })
        .await;
    assert_eq!(out, "foo");
}

#[tokio::test]
async fn both_values_together_now() {
    let (uint, str) = TEST_VALUE
        .scope(5, async {
            ANOTHER_TEST_VALUE
                .scope(String::from("foo"), async {
                    TEST_VALUE.with(|&uint| ANOTHER_TEST_VALUE.with(|str| (uint, str.clone())))
                })
                .await
        })
        .await;
    assert_eq!(uint, 5);
    assert_eq!(str, "foo");
}
