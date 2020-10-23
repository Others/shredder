use shredder::marker::GcDrop;
use shredder::*;

trait Node: Scan + ToScan + GcDrop {
    fn max_number(&self) -> Option<i64>;
    fn longest_string(&self) -> Option<String>;
}

#[derive(Scan)]
struct TreeNode(Gc<dyn Node>, Gc<dyn Node>);

impl Node for TreeNode {
    fn max_number(&self) -> Option<i64> {
        let lhs = self.0.get().max_number();
        let rhs = self.1.get().max_number();
        match (lhs, rhs) {
            (Some(lhs), Some(rhs)) => Some(i64::max(lhs, rhs)),
            (value, None) | (None, value) if value.is_some() => value,
            _ => None,
        }
    }

    fn longest_string(&self) -> Option<String> {
        let lhs = self.0.get();
        let rhs = self.1.get();

        match (lhs.longest_string(), rhs.longest_string()) {
            (Some(lhs), Some(rhs)) => {
                if lhs.len() > rhs.len() {
                    Some(lhs)
                } else {
                    Some(rhs)
                }
            }
            (value, None) | (None, value) if value.is_some() => value,
            _ => None,
        }
    }
}

#[derive(Scan)]
struct NumberNode(i64);

impl Node for NumberNode {
    fn max_number(&self) -> Option<i64> {
        Some(self.0)
    }

    fn longest_string(&self) -> Option<String> {
        None
    }
}

#[derive(Scan)]
struct StringNode(String);

impl Node for StringNode {
    fn max_number(&self) -> Option<i64> {
        None
    }
    fn longest_string(&self) -> Option<String> {
        Some(self.0.clone())
    }
}

macro_rules! make_node {
    ($node:expr) => {{
        Gc::from_box(Box::new($node))
    }};
}

#[test]
fn from_box() {
    run_with_gc_cleanup(|| {
        assert_eq!(number_of_tracked_allocations(), 0);

        let num1: Gc<dyn Node> = make_node!(NumberNode(10));
        let num2: Gc<dyn Node> = make_node!(NumberNode(100));
        let num3: Gc<dyn Node> = make_node!(NumberNode(1000));
        let str1: Gc<dyn Node> = make_node!(StringNode("this is a string".to_string()));
        let str2: Gc<dyn Node> = make_node!(StringNode("this is a longer string".to_string()));
        let str3: Gc<dyn Node> = make_node!(StringNode("this is the longest string".to_string()));

        assert_eq!(number_of_tracked_allocations(), 6);

        {
            let str_root: Gc<dyn Node> = make_node!(TreeNode(str1.clone(), str2.clone()));
            let num_root: Gc<dyn Node> = make_node!(TreeNode(num1.clone(), num2.clone()));
            let root: Gc<dyn Node> = make_node!(TreeNode(str_root, num_root));

            assert_eq!(number_of_tracked_allocations(), 9);

            assert_eq!(root.get().max_number().unwrap(), 100);
            assert_eq!(
                root.get().longest_string().unwrap(),
                "this is a longer string"
            );
        }

        collect();
        assert_eq!(number_of_tracked_allocations(), 6);

        {
            let mixed_root1: Gc<dyn Node> = make_node!(TreeNode(str1.clone(), num1.clone()));
            let mixed_root2: Gc<dyn Node> = make_node!(TreeNode(str2.clone(), num2.clone()));
            let mixed_root3: Gc<dyn Node> = make_node!(TreeNode(str3.clone(), num3.clone()));

            let mid_root: Gc<dyn Node> = make_node!(TreeNode(mixed_root1, mixed_root2));
            let root: Gc<dyn Node> = make_node!(TreeNode(mixed_root3, mid_root));

            assert_eq!(number_of_tracked_allocations(), 11);

            assert_eq!(root.get().max_number().unwrap(), 1000);
            assert_eq!(
                root.get().longest_string().unwrap(),
                "this is the longest string"
            );
        }

        collect();
        assert_eq!(number_of_tracked_allocations(), 6);
    });
}
