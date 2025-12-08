class CUP_MTVR_Base: Car_F {};
class CUP_MTVR_Refuel_Base: CUP_MTVR_Base
{
	transportFuel = 0;
	ace_refuel_hooks[] = {{-1.09, -0.01, -0.5},{1, -0.01, -0.5}};
	ace_refuel_fuelCargo = 10000;
};
class CUP_MTVR_Reammo_Base: CUP_MTVR_Base
{
	transportAmmo = 0;
	ace_rearm_defaultSupply = 1200;
};
class CUP_MTVR_Repair_Base: CUP_MTVR_Base
{
	transportRepair = 0;
	ace_repair_canRepair = 1;
};

class CUP_V3S_Open_Base: Car_F {};
class CUP_V3S_Refuel_Base: CUP_V3S_Open_Base
{
	transportFuel = 0;
	ace_refuel_hooks[] = {{-0.35, -3.35, -0.4},{0.40, -3.35, -0.4}};
	ace_refuel_fuelCargo = 6500;
};
class CUP_V3S_Repair_Base: CUP_V3S_Open_Base
{
	transportRepair = 0;
	ace_repair_canRepair = 1;
};
class CUP_V3S_Rearm_Base: CUP_V3S_Open_Base
{
	transportAmmo = 0;
	ace_rearm_defaultSupply = 1200;
};

class Truck_F: Car_F {};
class CUP_Ural_BaseTurret: Truck_F {};
class CUP_Ural_Support_Base: CUP_Ural_BaseTurret {};
class CUP_Ural_Refuel_Base: CUP_Ural_Support_Base
{
	transportFuel = 0;
	ace_refuel_hooks[] = {{-0.05, -3.65, -0.42}};
	ace_refuel_fuelCargo = 10000;
};
class CUP_Ural_Reammo_Base: CUP_Ural_Support_Base
{
	transportAmmo = 0;
	ace_rearm_defaultSupply = 1200;
};
class CUP_Ural_Repair_Base: CUP_Ural_Support_Base
{
	transportRepair = 0;
	ace_repair_canRepair = 1;
};

class Truck_02_base_F;
class Truck_02_fuel_base_F;
class Truck_02_box_base_F;
class CUP_Kamaz_5350_Base: Truck_02_base_F {};
class CUP_Kamaz_5350_Refuel_Base: Truck_02_fuel_base_F
{
	transportFuel = 0;
	ace_refuel_hooks[] = {{-0.02, -3.33, -1.05}};
	ace_refuel_fuelCargo = 10000;
};
class CUP_Kamaz_5350_ReAmmo_Base: CUP_Kamaz_5350_Base
{
	transportAmmo = 0;
	ace_rearm_defaultSupply = 1200;
};
class CUP_Kamaz_5350_Repair_Base: Truck_02_box_base_F
{
	transportRepair = 0;
	ace_repair_canRepair = 1;
};

class CUP_T810_Base: Car_F {};
class CUP_T810_Unarmed_Base: CUP_T810_Base {};
class CUP_T810_Refuel_Base: CUP_T810_Unarmed_Base
{
	transportFuel = 0;
	ace_refuel_hooks[] = {{-1.01, 0.21, -0.5},{1.08, 0.2, -0.5}};
	ace_refuel_fuelCargo = 10000;
};
class CUP_T810_Reammo_Base: CUP_T810_Unarmed_Base
{
	transportAmmo = 0;
	ace_rearm_defaultSupply = 1200;
};
class CUP_T810_Repair_Base: CUP_T810_Unarmed_Base
{
	transportRepair = 0;
	ace_repair_canRepair = 1;
};
