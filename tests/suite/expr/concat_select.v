// A bit-select, part-select or indexed part-select of a concatenation or a replication
// (IEEE 1800-2017 11.4.12): the bits of the concatenated value, its LSB at index 0.
module t;
  bit [7:0] b = 8'b10101100;
  bit [7:0] c = 8'b01010011;
  logic [3:0] x = 4'b1x0z;
  bit [3:0] a;
  integer i;
  initial begin
    #1;
    a = {b, c}[9:6];
    i = 7;
    if (a === 4'b0001 && {b, c}[15] === 1'b1 && {b, c}[0+:3] === 3'b011
        && {b, c}[i-:4] === 4'b0101 && {x, 4'b0011}[6:2] === 5'bx0z00
        && {3{2'b10}}[4:1] === 4'b0101)
      $display("PASSED");
    else $display("FAILED: %b %b %b", a, {b, c}[i-:4], {x, 4'b0011}[6:2]);
  end
endmodule
