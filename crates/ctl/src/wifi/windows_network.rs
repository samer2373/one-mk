unsafe fn network_changed(context: *const c_void) {
    if let Some(sender) = unsafe { context.cast::<mpsc::Sender<Action>>().as_ref() } { let _ = sender.send(Action::Refresh); }
}

unsafe extern "system" fn interface_changed(context: *const c_void, _row: *const MIB_IPINTERFACE_ROW, _kind: MIB_NOTIFICATION_TYPE) {
    unsafe { network_changed(context); }
}

unsafe extern "system" fn route_changed(context: *const c_void, _row: *const MIB_IPFORWARD_ROW2, _kind: MIB_NOTIFICATION_TYPE) {
    unsafe { network_changed(context); }
}

unsafe extern "system" fn address_changed(context: *const c_void, _row: *const MIB_UNICASTIPADDRESS_ROW, _kind: MIB_NOTIFICATION_TYPE) {
    unsafe { network_changed(context); }
}

pub(super) fn gateways() -> Vec<String> {
    let mut gateways = Vec::new();
    unsafe {
        let mut table = null_mut();
        if GetIpForwardTable2(AF_UNSPEC, &mut table) != ERROR_SUCCESS || table.is_null() { return gateways; }
        let routes = std::slice::from_raw_parts((*table).Table.as_ptr(), (*table).NumEntries as usize);
        for route in routes.iter().filter(|route| route.DestinationPrefix.PrefixLength == 0 && !route.Loopback) {
            let mut neighbor = MIB_IPNET_ROW2 { Address: route.NextHop, InterfaceIndex: route.InterfaceIndex, InterfaceLuid: route.InterfaceLuid, ..Default::default() };
            if (GetIpNetEntry2(&mut neighbor) != ERROR_SUCCESS || neighbor.PhysicalAddressLength != 6)
                && ResolveIpNetEntry2(&mut neighbor, null()) != ERROR_SUCCESS { continue; }
            if neighbor.PhysicalAddressLength == 6 {
                gateways.push(neighbor.PhysicalAddress[..6].iter().map(|byte| format!("{byte:02x}")).collect::<Vec<_>>().join(":"));
            }
        }
        FreeMibTable(table.cast());
    }
    gateways
}
