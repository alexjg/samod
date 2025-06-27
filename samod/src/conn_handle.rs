use futures::channel::mpsc;

pub(crate) struct ConnHandle {
    // Send messages to the outgoing channel (i.e. the network connection)
    pub(crate) tx: mpsc::UnboundedSender<Vec<u8>>,
    // Receive messages from the hub actor to the outbound connection
    //
    // The reason this is an `Option` is that we create it before the
    // `Samod::connect` future has picked it up. One the `connect` future
    // is running it will `take` the receiver
    pub(crate) rx: Option<mpsc::UnboundedReceiver<Vec<u8>>>,
}
