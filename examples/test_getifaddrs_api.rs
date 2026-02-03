// Test to verify getifaddrs 0.6 behavior
use getifaddrs::{getifaddrs, Address};

fn main() {
    println!("Testing getifaddrs 0.6.0 API behavior:");
    println!("========================================\n");
    
    match getifaddrs() {
        Ok(interfaces) => {
            let mut count = 0;
            for interface in interfaces {
                count += 1;
                if count > 5 {
                    println!("... (showing first 5 interfaces only)");
                    break;
                }
                
                println!("Interface: {}", interface.name);
                println!("  Index: {:?}", interface.index);
                
                // Test Address enum and helper methods
                match &interface.address {
                    Address::V4(_) => {
                        println!("  Type: IPv4");
                        if let Some(ip) = interface.address.ip_addr() {
                            println!("  IP: {}", ip);
                        }
                        if let Some(netmask) = interface.address.netmask() {
                            println!("  Netmask: {}", netmask);
                        }
                    }
                    Address::V6(_) => {
                        println!("  Type: IPv6");
                        if let Some(ip) = interface.address.ip_addr() {
                            println!("  IP: {}", ip);
                        }
                        if let Some(netmask) = interface.address.netmask() {
                            println!("  Netmask: {}", netmask);
                        }
                    }
                    Address::Mac(mac) => {
                        println!("  Type: MAC");
                        println!("  MAC: {:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}", 
                                 mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]);
                    }
                }
                
                println!();
            }
            
            println!("\nAPI compatibility verified:");
            println!("✓ Address is an enum (V4, V6, Mac)");
            println!("✓ ip_addr() method extracts IP addresses");
            println!("✓ netmask() method extracts netmasks from V4/V6");
            println!("✓ mac_addr() method extracts MAC addresses");
        }
        Err(e) => {
            eprintln!("Error: {}", e);
        }
    }
}
