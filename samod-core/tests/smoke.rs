use samod_test_harness::Network;

#[test]
fn basic_smoke() {
    let mut network = Network::new();
    let bob = network.create_samod("Bob");
    let alice = network.create_samod("Alice");

    network.connect(bob, alice);
    network.run_until_quiescent();
}
