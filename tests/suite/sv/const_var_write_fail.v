// exit: 1
// a procedural write of a const variable is an error (IEEE 1800-2017 6.20.6)
module top;
  initial begin
    const int b = 3;
    b++;
  end
endmodule
