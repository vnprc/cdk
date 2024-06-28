> **Warning**
> This project is in early development, it does however work with real sats! Always use amounts you don't mind loosing.


# Cashu Development Kit

CDK is a collection of rust crates for [Cashu](https://github.com/cashubtc) wallets and mints written in Rust.

**ALPHA** This library is in early development, the api will change and should be used with caution.


## Project structure

The project is split up into several crates in the `crates/` directory:

* Libraries:
    * [**cdk**](./crates/cdk/): Rust implementation of Cashu protocol.
    * [**cdk-sqlite**](./crates/cdk-sqlite/): Sqlite Storage backend
    * [**cdk-redb**](./crates/cdk-redb/): Redb Storage backend
* Binaries:
    * [**cdk-cli**](./crates/cdk-cli/): Cashu wallet CLI


## Implemented [NUTs](https://github.com/cashubtc/nuts/):

- :heavy_check_mark: [NUT-00](https://github.com/cashubtc/nuts/blob/main/00.md)
- :heavy_check_mark: [NUT-01](https://github.com/cashubtc/nuts/blob/main/01.md)
- :heavy_check_mark: [NUT-02](https://github.com/cashubtc/nuts/blob/main/02.md)
- :heavy_check_mark: [NUT-03](https://github.com/cashubtc/nuts/blob/main/03.md)
- :heavy_check_mark: [NUT-04](https://github.com/cashubtc/nuts/blob/main/04.md)
- :heavy_check_mark: [NUT-05](https://github.com/cashubtc/nuts/blob/main/05.md)
- :heavy_check_mark: [NUT-06](https://github.com/cashubtc/nuts/blob/main/06.md)
- :heavy_check_mark: [NUT-07](https://github.com/cashubtc/nuts/blob/main/07.md)
- :heavy_check_mark: [NUT-08](https://github.com/cashubtc/nuts/blob/main/08.md)
- :heavy_check_mark: [NUT-09](https://github.com/cashubtc/nuts/blob/main/09.md)
- :heavy_check_mark: [NUT-10](https://github.com/cashubtc/nuts/blob/main/10.md)
- :heavy_check_mark: [NUT-11](https://github.com/cashubtc/nuts/blob/main/11.md)
- :heavy_check_mark: [NUT-12](https://github.com/cashubtc/nuts/blob/main/12.md)
- :heavy_check_mark: [NUT-13](https://github.com/cashubtc/nuts/blob/main/13.md)
- :heavy_check_mark: [NUT-14](https://github.com/cashubtc/nuts/blob/main/14.md)
- :heavy_check_mark: [NUT-15](https://github.com/cashubtc/nuts/blob/main/15.md)

## Bindings

Experimental bindings can be found in the [bindings](./bindings/) folder.

## License

Code is under the [MIT License](LICENSE)

## Contribution

All contributions welcome.

Unless you explicitly state otherwise, any contribution intentionally submitted for inclusion in the work by you, shall be licensed as above, without any additional terms or conditions.
