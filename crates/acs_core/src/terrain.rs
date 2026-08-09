#[repr(usize)]
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Terrain {
    Null = 0,
    Soil,
    DepthWater,
    DDepthWater,
    ShallowWater,
    Mud,
    FertileSoil,
    LingSoil,
    WetLand,
    StoneLand,
    RockBrown,
    RockGray,
    RockMarble,
    IronOre,
    CopperOre,
    SilverOre,
    BornSpace,
    BornLine,
    Tmp1,
    Tmp2,
}
