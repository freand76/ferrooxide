// $readmemh / $readmemb of a missing file: the reference's error line, the array unchanged.
module t;
  reg [7:0] m [0:1];
  initial begin
    m[0] = 8'h12; m[1] = 8'h34;
    $readmemh("no_such_file_fx.mem", m);
    $readmemb("no_such_file_fx.mem", m);
    $display("%h %h", m[0], m[1]);
  end
endmodule
