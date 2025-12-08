class CfgVehicles
{
	class LandVehicle;
	class Car: LandVehicle
	{
		class ACE_Actions
		{
			class ACE_MainActions {};
		};
	};
	class Car_F: Car {};

	class Helicopter;
	class Helicopter_Base_F: Helicopter {};
	class Helicopter_Base_H: Helicopter_Base_F {};


	#include "Support.hpp"
	#include "Helicopters.hpp"
};
