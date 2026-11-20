// A constant array index with x bits reads x; a write through it is ignored.
module test;
  reg [3:0] tmp[1:2];
  real rr[0:1];
  reg [7:0] r;
  initial begin
    tmp[1] = 3;
    tmp[2] = 5;
    rr[0] = 1.5;
    r = {tmp['bx], tmp[1]};
    tmp['bx] = 7;
    $display("%h %h %h %f", r, tmp[1], tmp[2], rr['bx]);
  end
endmodule
