// Copyright 2021 The Grin Developers
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

//! Identifiers for various TUI elements, because they may be referenced
//! from a few different places

// Basic Status view
pub const VIEW_BASIC_STATUS: &str = "basic_status_view";

// Peer/Sync View
pub const VIEW_PEER_SYNC: &str = "peer_sync_view";
pub const TABLE_PEER_STATUS: &str = "peer_status_table";

// Mining View
pub const VIEW_MINING: &str = "mining_view";
pub const SUBMENU_MINING_BUTTON: &str = "mining_submenu_button";
pub const TABLE_MINING_STATUS: &str = "mining_status_table";
pub const TABLE_MINING_DIFF_STATUS: &str = "mining_diff_status_table";

// Logs View
pub const VIEW_LOGS: &str = "logs_view";

// Mining View
pub const VIEW_VERSION: &str = "version_view";

// Menu and root elements
pub const MAIN_MENU: &str = "main_menu";
pub const ROOT_STACK: &str = "root_stack";

// Logo (not final, to be used somewhere eventually
pub const _WELCOME_LOGO: &str = "                 GGGGG                      GGGGGGG         
               GGGGGGG                      GGGGGGGGG      
             GGGGGGGGG         GGGG         GGGGGGGGGG     
           GGGGGGGGGGG       GGGGGGGG       GGGGGGGGGGG    
          GGGGGGGGGGGG       GGGGGGGG       GGGGGGGGGGGG   
         GGGGGGGGGGGGG       GGGGGGGG       GGGGGGGGGGGGG  
        GGGGGGGGGGGGGG       GGGGGGGG       GGGGGGGGGGGGGG 
        GGGGGGGGGGGGGG       GGGGGGGGGGGGGGGGGGGGGGGGGGGGG 
       GGGGGGGGGGGGGGG       GGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
       GGGGGGGGGGGGGGG       GGGGGGGGGGGGGGGGGGGGGGGGGGGGGG
                             GGGGGG                        
                             GGGGGGG                       
                             GGGGGGGG                      
       GGGGGGGGGGGGGGG       GGGGGGGG       GGGGGGGGGGGGGGG
       GGGGGGGGGGGGGGG       GGGGGGGG       GGGGGGGGGGGGGGG
        GGGGGGGGGGGGGG       GGGGGGGG       GGGGGGGGGGGGGGG
         GGGGGGGGGGGGG       GGGGGGGG       GGGGGGGGGGGGGG 
          GGGGGGGGGGGG       GGGGGGGG       GGGGGGGGGGGGG  
           GGGGGGGGGGG       GGGGGGGG       GGGGGGGGGGGG   
            GGGGGGGGGG       GGGGGGGG       GGGGGGGGGGG    
              GGGGGGGG       GGGGGGGG       GGGGGGGGG      
               GGGGGGG       GGGGGGGG       GGGGGGG        
                  GGGG       GGGGGGGG       GGGG           
                    GG       GGGGGGGG       GG             
                             GGGGGGGG                       ";
