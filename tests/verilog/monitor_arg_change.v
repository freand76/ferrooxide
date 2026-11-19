// $monitor prints when the value of one of its arguments changes, not on any change of the
// signals they select from; $time, $stime and $realtime do not count; $monitoron prints
// the current values.
module top;
  reg [7:0] bs, ps;
  reg [7:0] mem [0:1];
  integer idx;
  initial begin
    bs = 0; ps = 0; mem[0] = 0;
    $monitor($time, " bs[1]=", bs[1], " ps[2:1]=", ps[2:1], " m=", mem[0][1], " t=%0t", $realtime);
    for (idx = 0; idx < 4; idx = idx + 1) #1 bs[idx] = 1'b1;
    for (idx = 0; idx < 4; idx = idx + 1) #1 ps[idx] = 1'b1;
    for (idx = 0; idx < 4; idx = idx + 1) #1 mem[0][idx] = 1'b1;
    #1 $monitoroff;
    #1 bs = 0;
    #1 $monitoron;
    #1 bs = 8'hff;
    #1 bs = 8'hfe;
  end
endmodule
