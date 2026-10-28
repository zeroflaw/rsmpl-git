struct Client;

impl Client {
    fn old_method(&self, arg: i32) -> i32 {
        arg + 1
    }
}

fn new_function(client: &Client, arg: i32) -> i32 {
    arg + 1
}

fn a(client: &Client) -> i32 {
    client.old_method(1)
}

fn b(client: &Client) -> i32 {
    client.old_method(2)
}

fn c(client: &Client) -> i32 {
    client.old_method(3)
}

fn d(client: &Client) -> i32 {
    client.old_method(4)
}
