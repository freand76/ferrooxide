// %s with NUL and unknown bytes; out-of-range string selects
module top;
  parameter id = "foo";
  reg [47:0] s;
  initial begin
    s = {8'b0, "a", 8'b0, "b", 8'bx, "c"};
    $display("[%s] [%0s]", s, s);
    s = {8'b0, 8'b0, "a", "b", "c", "d"};
    $display("[%s] [%0s] [%10s]", s, s, s);
    $display("[%0s] [%s]", id[8*10:8], id[8*10:8]);
  end
endmodule
