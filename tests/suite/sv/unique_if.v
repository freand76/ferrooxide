// expected
// unique / priority if: no true condition and no final else is reported (IEEE 1800 12.4.2);
// unique0 and chains with an else are silent
module t;
  reg [1:0] s;
  reg [3:0] y;
  always @(s) begin
    unique if (s == 0) y = 1;
    else if (s == 1) y = 2;
    priority if (s[1]) y = 3;
    unique0 if (s == 3) y = 4;
    unique if (s == 2) y = 5;
    else y = 6;
  end
  initial begin
    #1 s = 0;
    #1 s = 2;
    #1 s = 1;
    #1 $display("done y=%0d", y);
  end
endmodule
